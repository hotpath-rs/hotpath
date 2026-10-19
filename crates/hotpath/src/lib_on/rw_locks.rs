//! RwLock instrumentation module - tracks read/write lock acquisitions and hold durations.

use crossbeam_channel::{bounded, Receiver as CbReceiver, RecvTimeoutError, Sender as CbSender};
use hdrhistogram::Histogram;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::lib_on::{meta_rw_lock, MetaRwLock};

use crate::batch::{EventProducer, EventQueueRegistry};
use crate::instant::Instant;
use crate::lib_on::hotpath_guard::DRAIN_INTERVAL_MS;
use crate::lib_on::START_TIME;
use crate::metrics_server::METRICS_SERVER_PORT;

pub(crate) mod wrapper;

static RW_LOCK_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

fn next_rw_lock_id() -> u32 {
    RW_LOCK_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Whether an acquisition was a shared (read) or exclusive (write) lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RwLockKind {
    Read,
    Write,
}

/// Events sent to the background lock statistics collection thread.
#[derive(Debug)]
pub(crate) enum RwLockEvent {
    Created {
        id: u32,
        /// Column-including call-site key (`file:line:column`); distinguishes
        /// same-line invocations in the display-suffix scan. `source` is the
        /// `file:line` shown to users.
        key: &'static str,
        source: &'static str,
        label: Option<String>,
        type_name: &'static str,
    },
    /// Emitted when a guard is dropped. `wait_nanos` is the time blocked
    /// before the lock was granted; `acquire_nanos` is the held duration
    /// (granted -> released). Both `None` when timing was skipped under
    /// time sampling; the event still counts the lock.
    Released {
        id: u32,
        kind: RwLockKind,
        wait_nanos: Option<u64>,
        acquire_nanos: Option<u64>,
    },
}

/// Statistics for a single instrumented RwLock.
#[derive(Debug)]
pub(crate) struct RwLockEntry {
    pub(crate) id: u32,
    /// Column-including call-site key (`file:line:column`); the identity used
    /// by the display-suffix scan so same-line call sites do not cross-suffix.
    pub(crate) key: &'static str,
    /// The `file:line` form shown to users.
    pub(crate) source: &'static str,
    pub(crate) label: Option<String>,
    pub(crate) type_name: &'static str,
    pub(crate) read_count: u64,
    pub(crate) write_count: u64,
    pub(crate) read_sampled_count: u64,
    pub(crate) write_sampled_count: u64,
    pub(crate) read_wait_total_nanos: u64,
    pub(crate) write_wait_total_nanos: u64,
    pub(crate) read_acquire_total_nanos: u64,
    pub(crate) write_acquire_total_nanos: u64,
    read_wait_hist: Option<Histogram<u64>>,
    write_wait_hist: Option<Histogram<u64>>,
    read_acquire_hist: Option<Histogram<u64>>,
    write_acquire_hist: Option<Histogram<u64>>,
    pub(crate) iter: u32,
}

impl RwLockEntry {
    const LOW_NS: u64 = 1;
    const HIGH_NS: u64 = crate::lib_on::MAX_DURATION_NS;
    const SIGFIGS: u8 = 3;

    fn new_histogram() -> Histogram<u64> {
        Histogram::<u64>::new_with_bounds(Self::LOW_NS, Self::HIGH_NS, Self::SIGFIGS)
            .expect("hdrhistogram init")
    }

    #[inline]
    fn record(hist: &mut Option<Histogram<u64>>, nanos: u64) {
        if let Some(ref mut hist) = hist {
            hist.record(nanos.clamp(Self::LOW_NS, Self::HIGH_NS))
                .unwrap();
        }
    }

    pub(crate) fn count(&self, kind: RwLockKind) -> u64 {
        match kind {
            RwLockKind::Read => self.read_count,
            RwLockKind::Write => self.write_count,
        }
    }

    pub(crate) fn sampled_count(&self, kind: RwLockKind) -> u64 {
        match kind {
            RwLockKind::Read => self.read_sampled_count,
            RwLockKind::Write => self.write_sampled_count,
        }
    }

    pub(crate) fn wait_total_nanos(&self, kind: RwLockKind) -> u64 {
        match kind {
            RwLockKind::Read => self.read_wait_total_nanos,
            RwLockKind::Write => self.write_wait_total_nanos,
        }
    }

    pub(crate) fn acquire_total_nanos(&self, kind: RwLockKind) -> u64 {
        match kind {
            RwLockKind::Read => self.read_acquire_total_nanos,
            RwLockKind::Write => self.write_acquire_total_nanos,
        }
    }

    pub(crate) fn wait_avg_nanos(&self, kind: RwLockKind) -> u64 {
        self.wait_total_nanos(kind)
            .checked_div(self.sampled_count(kind))
            .unwrap_or(0)
    }

    pub(crate) fn acquire_avg_nanos(&self, kind: RwLockKind) -> u64 {
        self.acquire_total_nanos(kind)
            .checked_div(self.sampled_count(kind))
            .unwrap_or(0)
    }

    fn percentile(hist: &Option<Histogram<u64>>, count: u64, p: f64) -> u64 {
        match hist {
            Some(hist) if count > 0 => hist.value_at_percentile(p.clamp(0.0, 100.0)),
            _ => 0,
        }
    }

    pub(crate) fn wait_percentile_nanos(&self, kind: RwLockKind, p: f64) -> u64 {
        let hist = match kind {
            RwLockKind::Read => &self.read_wait_hist,
            RwLockKind::Write => &self.write_wait_hist,
        };
        Self::percentile(hist, self.sampled_count(kind), p)
    }

    pub(crate) fn acquire_percentile_nanos(&self, kind: RwLockKind, p: f64) -> u64 {
        let hist = match kind {
            RwLockKind::Read => &self.read_acquire_hist,
            RwLockKind::Write => &self.write_acquire_hist,
        };
        Self::percentile(hist, self.sampled_count(kind), p)
    }

    /// Bucket projections of the sampled wait/acquire durations of one lock
    /// side for the Prometheus exporter (sparse native at `schema`, cumulative
    /// classic on `boundaries`).
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn native_wait_buckets(&self, kind: RwLockKind, schema: i32) -> Vec<(i32, u64)> {
        let hist = match kind {
            RwLockKind::Read => &self.read_wait_hist,
            RwLockKind::Write => &self.write_wait_hist,
        };
        crate::lib_on::native_histograms::native_buckets_opt(
            hist.as_ref(),
            self.sampled_count(kind) > 0,
            schema,
            crate::lib_on::native_histograms::NANOS_SCALE,
        )
    }

    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn classic_wait_buckets(&self, kind: RwLockKind, boundaries: &[u64]) -> Vec<u64> {
        let hist = match kind {
            RwLockKind::Read => &self.read_wait_hist,
            RwLockKind::Write => &self.write_wait_hist,
        };
        crate::lib_on::native_histograms::classic_buckets_opt(
            hist.as_ref(),
            self.sampled_count(kind) > 0,
            boundaries,
        )
    }

    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn native_acquire_buckets(&self, kind: RwLockKind, schema: i32) -> Vec<(i32, u64)> {
        let hist = match kind {
            RwLockKind::Read => &self.read_acquire_hist,
            RwLockKind::Write => &self.write_acquire_hist,
        };
        crate::lib_on::native_histograms::native_buckets_opt(
            hist.as_ref(),
            self.sampled_count(kind) > 0,
            schema,
            crate::lib_on::native_histograms::NANOS_SCALE,
        )
    }

    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn classic_acquire_buckets(&self, kind: RwLockKind, boundaries: &[u64]) -> Vec<u64> {
        let hist = match kind {
            RwLockKind::Read => &self.read_acquire_hist,
            RwLockKind::Write => &self.write_acquire_hist,
        };
        crate::lib_on::native_histograms::classic_buckets_opt(
            hist.as_ref(),
            self.sampled_count(kind) > 0,
            boundaries,
        )
    }
}

pub(crate) struct RwLocksInternalState {
    pub(crate) stats: HashMap<u32, RwLockEntry>,
}

pub(crate) struct RwLocksState {
    pub(crate) inner: Arc<MetaRwLock<RwLocksInternalState>>,
    pub(crate) shutdown_tx: Mutex<Option<CbSender<()>>>,
    pub(crate) completion_rx: Mutex<Option<CbReceiver<()>>>,
}

pub(crate) static RW_LOCKS_STATE: OnceLock<RwLocksState> = OnceLock::new();

/// Runs `f` on the entries sorted for display, borrowed under the read lock:
/// nothing is cloned, so the per-entry histograms stay in the map.
pub(crate) fn with_sorted_rw_lock_entries<R>(f: impl FnOnce(&[&RwLockEntry]) -> R) -> R {
    let Some(state) = RW_LOCKS_STATE.get() else {
        return f(&[]);
    };
    let guard = state.inner.read().unwrap();
    let mut stats: Vec<&RwLockEntry> = guard.stats.values().collect();
    stats.sort_by(|a, b| compare_rw_lock_entries(a, b));
    f(&stats)
}

pub(crate) fn get_rw_locks_json() -> crate::json::JsonRwLocksList {
    let elapsed = std::time::Duration::from_nanos(crate::lib_on::current_elapsed_ns());
    let percentiles = crate::lib_on::hotpath_guard::configured_percentiles();
    with_sorted_rw_lock_entries(|entries| {
        crate::lib_on::report::collect_rw_locks_json(entries, 0, elapsed, &percentiles, false)
    })
}

#[inline]
pub(crate) fn elapsed_nanos(start: Instant) -> u64 {
    start.elapsed().as_nanos() as u64
}

/// One sampling decision per acquisition; `None` skips both clock read pairs.
#[inline]
pub(crate) fn wait_stamp() -> Option<Instant> {
    crate::lib_on::sampling::rw_locks_should_time().then(Instant::now)
}

static EVENT_QUEUES: EventQueueRegistry<RwLockEvent> = EventQueueRegistry::new();

thread_local! {
    static EVENT_PRODUCER: EventProducer<RwLockEvent> = EVENT_QUEUES.register();
}

#[inline]
pub(crate) fn send_rw_lock_event(event: RwLockEvent) {
    if !EVENT_QUEUES.is_active() {
        return;
    }
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let _ = EVENT_PRODUCER.try_with(|producer| producer.push(event));
}

/// Stops producers ahead of the worker's final sweep at shutdown.
pub(crate) fn stop_rw_lock_events() {
    EVENT_QUEUES.set_active(false);
}

/// Entry for events that arrive ahead of their `Created` (sweeps only preserve
/// per-thread order, so another thread's data events can be drained first).
/// `Created` backfills the metadata.
fn placeholder_rw_lock_entry(id: u32) -> RwLockEntry {
    RwLockEntry {
        id,
        key: "",
        source: "",
        label: None,
        type_name: "",
        read_count: 0,
        write_count: 0,
        read_sampled_count: 0,
        write_sampled_count: 0,
        read_wait_total_nanos: 0,
        write_wait_total_nanos: 0,
        read_acquire_total_nanos: 0,
        write_acquire_total_nanos: 0,
        read_wait_hist: Some(RwLockEntry::new_histogram()),
        write_wait_hist: Some(RwLockEntry::new_histogram()),
        read_acquire_hist: Some(RwLockEntry::new_histogram()),
        write_acquire_hist: Some(RwLockEntry::new_histogram()),
        iter: 0,
    }
}

fn process_rw_lock_event(state: &mut RwLocksInternalState, event: RwLockEvent) {
    match event {
        RwLockEvent::Created {
            id,
            key,
            source,
            label,
            type_name,
        } => {
            let iter = state.stats.values().filter(|s| s.key == key).count() as u32;
            let entry = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_rw_lock_entry(id));
            entry.key = key;
            entry.source = source;
            entry.label = label;
            entry.type_name = type_name;
            entry.iter = iter;
        }
        RwLockEvent::Released {
            id,
            kind,
            wait_nanos,
            acquire_nanos,
        } => {
            let entry = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_rw_lock_entry(id));
            let sampled = match (wait_nanos, acquire_nanos) {
                (Some(wait), Some(acquire)) => Some((wait, acquire)),
                _ => None,
            };
            match kind {
                RwLockKind::Read => {
                    entry.read_count += 1;
                    if let Some((wait_nanos, acquire_nanos)) = sampled {
                        entry.read_sampled_count += 1;
                        entry.read_wait_total_nanos += wait_nanos;
                        entry.read_acquire_total_nanos += acquire_nanos;
                        RwLockEntry::record(&mut entry.read_wait_hist, wait_nanos);
                        RwLockEntry::record(&mut entry.read_acquire_hist, acquire_nanos);
                    }
                }
                RwLockKind::Write => {
                    entry.write_count += 1;
                    if let Some((wait_nanos, acquire_nanos)) = sampled {
                        entry.write_sampled_count += 1;
                        entry.write_wait_total_nanos += wait_nanos;
                        entry.write_acquire_total_nanos += acquire_nanos;
                        RwLockEntry::record(&mut entry.write_wait_hist, wait_nanos);
                        RwLockEntry::record(&mut entry.write_acquire_hist, acquire_nanos);
                    }
                }
            }
        }
    }
}

/// Registers a new RwLock with the profiling subsystem.
pub(crate) fn register_rw_lock<T>(key: &'static str, label: Option<String>) -> u32 {
    let type_name = std::any::type_name::<T>();
    let source = crate::channels::display_source(key);
    init_rw_locks_state();
    let id = next_rw_lock_id();

    send_rw_lock_event(RwLockEvent::Created {
        id,
        key,
        source,
        label,
        type_name,
    });

    id
}

fn flush_rw_lock_buffer(
    buffer: &mut Vec<RwLockEvent>,
    inner: &Arc<MetaRwLock<RwLocksInternalState>>,
) {
    if buffer.is_empty() {
        return;
    }
    if let Ok(mut shared) = inner.write() {
        for e in buffer.drain(..) {
            process_rw_lock_event(&mut shared, e);
        }
    }
}

/// Initialize the lock statistics collection system (called on first instrumented lock).
pub(crate) fn init_rw_locks_state() -> &'static RwLocksState {
    RW_LOCKS_STATE.get_or_init(|| {
        START_TIME.get_or_init(Instant::now);

        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let (completion_tx, completion_rx) = bounded::<()>(1);

        let inner = Arc::new(meta_rw_lock!(
            "rw_locks_state",
            RwLocksInternalState {
                stats: HashMap::new(),
            },
        ));
        let inner_clone = Arc::clone(&inner);

        EVENT_QUEUES.set_active(true);

        crate::lib_on::spawn_profiler_thread("hp-rw-locks", move || {
            let flush_interval = std::time::Duration::from_millis(*DRAIN_INTERVAL_MS);
            let mut swept: Vec<RwLockEvent> = Vec::new();

            // Single consumer of the per-thread event queues: capped sweep on
            // every tick, then one uncapped drain when shutdown is signalled
            // (producers are already stopped by then, so nothing is left behind).
            loop {
                let shutdown = !matches!(
                    shutdown_rx.recv_timeout(flush_interval),
                    Err(RecvTimeoutError::Timeout)
                );

                if shutdown {
                    EVENT_QUEUES.drain_all(&mut swept);
                    flush_rw_lock_buffer(&mut swept, &inner_clone);
                    break;
                }

                EVENT_QUEUES.sweep(&mut swept);
                flush_rw_lock_buffer(&mut swept, &inner_clone);
            }

            let _ = completion_tx.send(());
        })
        .expect("Failed to spawn rw_lock-stats-collector thread");

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        RwLocksState {
            inner,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            completion_rx: Mutex::new(Some(completion_rx)),
        }
    })
}

/// Compare two lock stats for sorting. Custom labels first, then by source and iter.
pub(crate) fn compare_rw_lock_entries(a: &RwLockEntry, b: &RwLockEntry) -> std::cmp::Ordering {
    match (a.label.is_some(), b.label.is_some()) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (true, true) => a
            .label
            .as_ref()
            .unwrap()
            .cmp(b.label.as_ref().unwrap())
            .then_with(|| a.iter.cmp(&b.iter)),
        (false, false) => a.source.cmp(b.source).then_with(|| a.iter.cmp(&b.iter)),
    }
}

/// Trait for instrumenting RwLocks. Dispatches on the type of the wrapped lock
/// (e.g. [`std::sync::RwLock`] or `parking_lot::RwLock`).
///
/// This trait is not intended for direct use. Use the `rw_lock!` macro instead.
#[doc(hidden)]
pub trait InstrumentRwLock {
    type Output;
    fn instrument(self, source: &'static str, label: Option<String>) -> Self::Output;
}

/// Instrument an [`std::sync::RwLock`], `parking_lot::RwLock`, `async_lock::RwLock`, or
/// `tokio::sync::RwLock` for read/write profiling.
///
/// Returns an instrumented drop-in replacement that proxies to the wrapped lock and records
/// how long read and write locks are held. The wrapper type matches the API of the underlying
/// lock (`std::sync::RwLock` returns `LockResult`s; `parking_lot::RwLock` returns guards directly;
/// `async_lock::RwLock` and `tokio::sync::RwLock` expose async `read`/`write` returning guards).
///
/// `parking_lot::RwLock` support requires the `parking_lot` feature; `async_lock::RwLock`
/// support requires the `async-lock` feature; `tokio::sync::RwLock` support requires the
/// `tokio` feature.
///
/// # Examples
///
/// ```rust,no_run
/// let lock = hotpath::rw_lock!(std::sync::RwLock::new(0u32));
/// *lock.write().unwrap() += 1;
/// let _ = *lock.read().unwrap();
/// ```
#[macro_export]
macro_rules! rw_lock {
    ($expr:expr) => {{
        const RW_LOCK_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(RW_LOCK_ID);
        $crate::InstrumentRwLock::instrument($expr, RW_LOCK_ID, None)
    }};

    // A literal label is checked for uniqueness, then re-dispatched wrapped in
    // a block so it takes the plain expression arm below.
    ($expr:expr, label = $label:literal) => {{
        $crate::__unique_label!(rw_lock, $label);
        $crate::rw_lock!($expr, label = { $label })
    }};

    ($expr:expr, label = $label:expr) => {{
        const RW_LOCK_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(RW_LOCK_ID);
        $crate::InstrumentRwLock::instrument($expr, RW_LOCK_ID, Some($label.to_string()))
    }};
}
