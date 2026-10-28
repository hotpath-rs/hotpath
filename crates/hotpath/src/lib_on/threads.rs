//! Thread monitoring: CPU usage statistics for all threads in the current process.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, OnceLock};

use crate::lib_on::{meta_rw_lock, MetaRwLock};
use std::time::Duration;

use crate::instant::Instant;

#[cfg(target_os = "macos")]
#[path = "threads/collector_macos.rs"]
mod collector;

#[cfg(target_os = "linux")]
#[path = "threads/collector_linux.rs"]
mod collector;

#[cfg(target_os = "windows")]
#[path = "threads/collector_windows.rs"]
mod collector;

pub(crate) use crate::json::ThreadMetrics;
use crate::json::{JsonThreadEntry, JsonThreadsList};
use crate::output::Precision;

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn thread_metrics_with_percentage(
    mut metrics: ThreadMetrics,
    prev: Option<&ThreadMetrics>,
    elapsed_secs: f64,
) -> ThreadMetrics {
    if let Some(prev_metrics) = prev {
        if prev_metrics.os_tid == metrics.os_tid && elapsed_secs > 0.0 {
            let cpu_delta = metrics.cpu_total - prev_metrics.cpu_total;
            metrics.cpu_percent = Some((cpu_delta / elapsed_secs) * 100.0);
        }
    }
    metrics
}

#[allow(dead_code)]
struct ThreadsState {
    /// Last sampled metrics for CPU percentage calculation
    previous_metrics: HashMap<u64, ThreadMetrics>,
    /// Current metrics snapshot (live threads only)
    current_metrics: Vec<ThreadMetrics>,
    last_sample_time: Instant,
    sample_interval: Duration,
    /// Start time for elapsed calculation
    start_time: Instant,
    /// Peak CPU percentage per thread (keyed by os_tid)
    max_cpu_percent: HashMap<u64, f64>,
    /// Per-thread baseline (cpu_total, timestamp) at first observation, so
    /// cpu_percent_avg excludes CPU accumulated before profiler started.
    baseline_cpu: HashMap<u64, (f64, Instant)>,
}

type ThreadsStateRef = Arc<MetaRwLock<ThreadsState>>;

static THREADS_STATE: OnceLock<ThreadsStateRef> = OnceLock::new();

static THREADS_INTERVAL_MS: LazyLock<u64> = LazyLock::new(|| {
    std::env::var("HOTPATH_THREADS_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(250)
});

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn init_threads_monitoring() {
    THREADS_STATE.get_or_init(|| {
        let sample_interval_ms = *THREADS_INTERVAL_MS;

        let sample_interval = Duration::from_millis(sample_interval_ms);
        let start_time = Instant::now();

        let state = Arc::new(meta_rw_lock!(
            "threads_state",
            ThreadsState {
                previous_metrics: HashMap::new(),
                current_metrics: Vec::new(),
                last_sample_time: start_time,
                sample_interval,
                start_time,
                max_cpu_percent: HashMap::new(),
                baseline_cpu: HashMap::new(),
            },
        ));

        let state_clone = Arc::clone(&state);

        std::thread::Builder::new()
            .name("hp-threads".into())
            .spawn(move || {
                let _suspend = crate::lib_on::SuspendAllocTracking::new();
                collector_loop(state_clone, sample_interval);
            })
            .expect("Failed to spawn thread-metrics-collector thread");

        state
    });
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn collector_loop(state: ThreadsStateRef, interval: Duration) {
    loop {
        match collector::collect_thread_metrics() {
            Ok(raw_metrics) => {
                let mut state_guard = match state.write() {
                    Ok(guard) => guard,
                    Err(_) => continue,
                };
                let elapsed_secs = state_guard.last_sample_time.elapsed().as_secs_f64();

                // Calculate CPU percentages by comparing with previous sample
                let mut new_metrics = Vec::with_capacity(raw_metrics.len());
                for metric in raw_metrics {
                    // Cumulative CPU time can never decrease for the same
                    // thread, so a drop means the tid was recycled by a new
                    // thread - reset its per-tid sampling state.
                    let tid_reused = state_guard
                        .previous_metrics
                        .get(&metric.os_tid)
                        .is_some_and(|prev| metric.cpu_total < prev.cpu_total);
                    if tid_reused {
                        state_guard.previous_metrics.remove(&metric.os_tid);
                        state_guard.max_cpu_percent.remove(&metric.os_tid);
                        state_guard.baseline_cpu.remove(&metric.os_tid);
                    }

                    let prev = state_guard.previous_metrics.get(&metric.os_tid);
                    #[allow(unused_mut)]
                    let mut m_with_percent =
                        thread_metrics_with_percentage(metric, prev, elapsed_secs);

                    // A thread that already exited can still be enumerated as a
                    // zombie port with its pthread (and name) gone - fall back
                    // to the name its allocator registration hook captured, or
                    // the name it was last seen alive with.
                    if m_with_percent.name.starts_with("thread_") {
                        #[cfg(feature = "hotpath-alloc")]
                        if let Some(name) =
                            crate::lib_on::functions::alloc::core::get_registered_thread_name(
                                m_with_percent.os_tid,
                            )
                        {
                            m_with_percent.name = name;
                        }
                        if let Some(prev) = prev {
                            if m_with_percent.name.starts_with("thread_")
                                && !prev.name.starts_with("thread_")
                            {
                                m_with_percent.name = prev.name.clone();
                            }
                        }
                    }

                    // Merge per-thread allocation stats
                    #[cfg(feature = "hotpath-alloc")]
                    if let Some((alloc, dealloc)) =
                        crate::lib_on::functions::alloc::core::get_thread_alloc_stats(
                            m_with_percent.os_tid,
                        )
                    {
                        m_with_percent.alloc_bytes = Some(alloc);
                        m_with_percent.dealloc_bytes = Some(dealloc);
                        m_with_percent.mem_diff = Some(alloc as i64 - dealloc as i64);
                    }

                    let now = Instant::now();
                    let (baseline_cpu, baseline_at) = *state_guard
                        .baseline_cpu
                        .entry(m_with_percent.os_tid)
                        .or_insert((m_with_percent.cpu_total, now));
                    let observed_secs = now.duration_since(baseline_at).as_secs_f64();
                    if observed_secs > 0.0 {
                        let cpu_since_baseline = m_with_percent.cpu_total - baseline_cpu;
                        m_with_percent.cpu_percent_avg =
                            Some((cpu_since_baseline / observed_secs) * 100.0);
                    }

                    if let Some(pct) = m_with_percent.cpu_percent {
                        let max = state_guard
                            .max_cpu_percent
                            .entry(m_with_percent.os_tid)
                            .or_insert(0.0);
                        if pct > *max {
                            *max = pct;
                        }
                        m_with_percent.cpu_percent_max = Some(*max);
                    } else if let Some(&max) =
                        state_guard.max_cpu_percent.get(&m_with_percent.os_tid)
                    {
                        m_with_percent.cpu_percent_max = Some(max);
                    }

                    new_metrics.push(m_with_percent);
                }

                // Drop per-tid sampling state of threads that disappeared, so
                // these maps stay bounded and a recycled tid starts fresh.
                let live_tids: HashSet<u64> = new_metrics.iter().map(|m| m.os_tid).collect();
                state_guard
                    .max_cpu_percent
                    .retain(|tid, _| live_tids.contains(tid));
                state_guard
                    .baseline_cpu
                    .retain(|tid, _| live_tids.contains(tid));

                state_guard.previous_metrics =
                    new_metrics.iter().map(|m| (m.os_tid, m.clone())).collect();
                state_guard.current_metrics = new_metrics;
                state_guard.last_sample_time = Instant::now();
            }
            Err(e) => {
                eprintln!("[hotpath] Failed to collect thread metrics: {}", e);
            }
        }

        std::thread::sleep(interval);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn collector_loop(_state: ThreadsStateRef, _interval: Duration) {
    // No-op on unsupported platforms - sleep forever
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

/// Get RSS from collector (platform-specific)
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn get_rss_bytes() -> Option<u64> {
    collector::get_rss_bytes()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn get_rss_bytes() -> Option<u64> {
    None
}

/// Max RSS over the whole program lifetime (not just since the guard was
/// built), read from the kernel's high-water mark, so no sample can miss it.
/// Excludes a launcher that exec'd into the program, such as `cargo run`.
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn get_rss_bytes_max() -> Option<u64> {
    collector::get_rss_bytes_max()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn get_rss_bytes_max() -> Option<u64> {
    None
}

/// Raw thread snapshot behind both the JSON list and the Prometheus
/// exporter: live sampled metrics with per-thread allocation stats joined in.
pub(crate) struct ThreadsRaw {
    pub(crate) metrics: Vec<ThreadMetrics>,
    /// Threads in the most recent monitor sample - the first `live_count`
    /// rows of `metrics`. The rows after them are joined in from the
    /// allocation registry (exited or unsampled threads) so allocation totals
    /// are never lost, and must not count toward a live-thread gauge. Only
    /// the Prometheus exporter distinguishes the two spans.
    #[cfg_attr(not(feature = "hotpath-prometheus"), allow(dead_code))]
    pub(crate) live_count: usize,
    pub(crate) rss_bytes: Option<u64>,
    pub(crate) rss_bytes_max: Option<u64>,
    pub(crate) current_elapsed_ns: u64,
    pub(crate) sample_interval_ms: u64,
    /// Bytes allocated/deallocated by threads that never got an allocation
    /// registry slot (registry full). Counted only in process-wide totals;
    /// zero without `hotpath-alloc`.
    pub(crate) overflow_alloc_bytes: u64,
    pub(crate) overflow_dealloc_bytes: u64,
}

/// `None` until the thread monitor has started.
pub(crate) fn get_threads_raw() -> Option<ThreadsRaw> {
    let rss_bytes = get_rss_bytes();
    let rss_bytes_max = get_rss_bytes_max();
    let state = THREADS_STATE.get()?;
    let state_guard = state.read().ok()?;
    let current_elapsed_ns = state_guard.start_time.elapsed().as_nanos() as u64;

    #[allow(unused_mut)]
    let mut current_metrics = state_guard.current_metrics.clone();
    let live_count = current_metrics.len();

    // The allocation registry is the single source of truth for
    // per-thread allocation stats. Live sampled rows join their alloc
    // columns from it; registry entries without a live row (threads
    // that exited, or that allocated but were not sampled yet) become
    // rows of their own, so allocation stats are never lost. Their
    // status comes from a liveness probe; their CPU stats are unknown.
    #[cfg(feature = "hotpath-alloc")]
    {
        use crate::lib_on::functions::alloc::core::{
            get_registered_thread_name, get_registered_thread_stats, get_thread_alloc_stats,
            is_orphaned_tid,
        };

        for m in &mut current_metrics {
            // A sample can catch a thread before std names it, and a short
            // run may never resample it - the registry name is authoritative.
            if let Some(name) = get_registered_thread_name(m.os_tid) {
                m.name = name;
            }
            if let Some((alloc, dealloc)) = get_thread_alloc_stats(m.os_tid) {
                m.alloc_bytes = Some(alloc);
                m.dealloc_bytes = Some(dealloc);
                m.mem_diff = Some(alloc as i64 - dealloc as i64);
            }
        }

        let live_tids: HashSet<u64> = current_metrics.iter().map(|m| m.os_tid).collect();
        for (tid, alloc, dealloc, name) in get_registered_thread_stats() {
            if live_tids.contains(&tid) {
                continue;
            }
            let status = if is_orphaned_tid(tid) {
                "Exited"
            } else {
                match collector::is_thread_alive(tid) {
                    Some(false) => "Exited",
                    _ => "Unsampled",
                }
            };
            let name = name.unwrap_or_else(|| {
                if is_orphaned_tid(tid) {
                    "(recycled tid)".to_string()
                } else {
                    format!("thread_{tid}")
                }
            });
            let mut m = ThreadMetrics::new(tid, name, status.to_string(), String::new(), 0.0, 0.0);
            m.alloc_bytes = Some(alloc);
            m.dealloc_bytes = Some(dealloc);
            m.mem_diff = Some(alloc as i64 - dealloc as i64);
            current_metrics.push(m);
        }
    }

    #[cfg(feature = "hotpath-alloc")]
    let (overflow_alloc_bytes, overflow_dealloc_bytes) =
        crate::lib_on::functions::alloc::core::get_overflow_alloc_stats();
    #[cfg(not(feature = "hotpath-alloc"))]
    let (overflow_alloc_bytes, overflow_dealloc_bytes) = (0u64, 0u64);

    Some(ThreadsRaw {
        metrics: current_metrics,
        live_count,
        rss_bytes,
        rss_bytes_max,
        current_elapsed_ns,
        sample_interval_ms: state_guard.sample_interval.as_millis() as u64,
        overflow_alloc_bytes,
        overflow_dealloc_bytes,
    })
}

/// Get current thread metrics as JSON. `precision` applies to the byte
/// fields, process and per-thread, which the cloud diff reads back; CPU
/// percentages stay in display form.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_threads_json(precision: Precision) -> JsonThreadsList {
    let Some(raw) = get_threads_raw() else {
        return JsonThreadsList {
            current_elapsed_ns: 0,
            sample_interval_ms: *THREADS_INTERVAL_MS,
            data: Vec::new(),
            total_count: 0,
            included_count: 0,
            thread_count: 0,
            rss_bytes: get_rss_bytes().map(|b| precision.bytes(b)),
            rss_bytes_max: get_rss_bytes_max().map(|b| precision.bytes(b)),
            total_alloc_bytes: None,
            total_dealloc_bytes: None,
            alloc_dealloc_diff: None,
        };
    };
    let current_metrics = raw.metrics;

    let (total_alloc, total_dealloc) = current_metrics.iter().fold(
        (raw.overflow_alloc_bytes, raw.overflow_dealloc_bytes),
        |(alloc, dealloc), m| {
            (
                alloc + m.alloc_bytes.unwrap_or(0),
                dealloc + m.dealloc_bytes.unwrap_or(0),
            )
        },
    );

    let has_alloc_data = current_metrics.iter().any(|m| m.alloc_bytes.is_some());

    let (total_alloc_bytes, total_dealloc_bytes, alloc_dealloc_diff) = if has_alloc_data {
        let diff = total_alloc as i64 - total_dealloc as i64;
        (
            Some(precision.bytes(total_alloc)),
            Some(precision.bytes(total_dealloc)),
            Some(precision.bytes_signed(diff)),
        )
    } else {
        (None, None, None)
    };

    let mut sorted_metrics: Vec<&ThreadMetrics> = current_metrics.iter().collect();

    #[cfg(feature = "hotpath-alloc")]
    sorted_metrics.sort_by_key(|m| {
        std::cmp::Reverse(m.alloc_bytes.unwrap_or(0).max(m.dealloc_bytes.unwrap_or(0)))
    });

    #[cfg(not(feature = "hotpath-alloc"))]
    sorted_metrics.sort_by(|a, b| {
        b.cpu_percent_max
            .unwrap_or(0.0)
            .total_cmp(&a.cpu_percent_max.unwrap_or(0.0))
    });

    JsonThreadsList {
        current_elapsed_ns: raw.current_elapsed_ns,
        sample_interval_ms: raw.sample_interval_ms,
        data: sorted_metrics
            .iter()
            .map(|m| thread_entry(m, precision))
            .collect(),
        total_count: sorted_metrics.len(),
        included_count: sorted_metrics.len(),
        thread_count: current_metrics.len(),
        rss_bytes: raw.rss_bytes.map(|b| precision.bytes(b)),
        rss_bytes_max: raw.rss_bytes_max.map(|b| precision.bytes(b)),
        total_alloc_bytes,
        total_dealloc_bytes,
        alloc_dealloc_diff,
    }
}

fn thread_entry(metrics: &ThreadMetrics, precision: Precision) -> JsonThreadEntry {
    JsonThreadEntry {
        os_tid: metrics.os_tid,
        name: metrics.name.clone(),
        status: metrics.status.clone(),
        status_code: metrics.status_code.clone(),
        cpu_percent: metrics.cpu_percent.map(|p| format!("{:.1}%", p)),
        cpu_percent_max: metrics.cpu_percent_max.map(|p| format!("{:.1}%", p)),
        cpu_percent_avg: metrics.cpu_percent_avg.map(|p| format!("{:.1}%", p)),
        alloc_bytes: metrics.alloc_bytes.map(|b| precision.bytes(b)),
        dealloc_bytes: metrics.dealloc_bytes.map(|b| precision.bytes(b)),
        mem_diff: metrics.mem_diff.map(|b| precision.bytes_signed(b)),
    }
}
