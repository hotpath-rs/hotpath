//! Mutex instrumentation module - tracks lock acquisitions and hold durations.
//!
//! Unlike [`crate::rw_locks`], a mutex has a single lock kind (no read/write
//! distinction), so each entry tracks one set of wait & acquire statistics.

use crossbeam_channel::{bounded, Receiver as CbReceiver, RecvTimeoutError, Sender as CbSender};
use hdrhistogram::Histogram;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use crate::lib_on::{meta_rw_lock, MetaRwLock};

use crate::batch::{EventProducer, EventQueueRegistry};
use crate::instant::Instant;
use crate::lib_on::hotpath_guard::DRAIN_INTERVAL_MS;
use crate::lib_on::START_TIME;
use crate::metrics_server::METRICS_SERVER_PORT;

pub(crate) mod wrapper;

static MUTEX_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

fn next_mutex_id() -> u32 {
    MUTEX_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Events sent to the background lock statistics collection thread.
#[derive(Debug)]
pub(crate) enum MutexEvent {
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
        wait_nanos: Option<u64>,
        acquire_nanos: Option<u64>,
    },
}

/// Statistics for a single instrumented Mutex.
#[derive(Debug)]
pub(crate) struct MutexEntry {
    pub(crate) id: u32,
    /// Column-including call-site key (`file:line:column`); the identity used
    /// by the display-suffix scan so same-line call sites do not cross-suffix.
    pub(crate) key: &'static str,
    /// The `file:line` form shown to users.
    pub(crate) source: &'static str,
    pub(crate) label: Option<String>,
    pub(crate) type_name: &'static str,
    pub(crate) count: u64,
    pub(crate) sampled_count: u64,
    pub(crate) wait_total_nanos: u64,
    pub(crate) acquire_total_nanos: u64,
    wait_hist: Option<Histogram<u64>>,
    acquire_hist: Option<Histogram<u64>>,
    pub(crate) iter: u32,
}

impl MutexEntry {
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

    pub(crate) fn wait_avg_nanos(&self) -> u64 {
        self.wait_total_nanos
            .checked_div(self.sampled_count)
            .unwrap_or(0)
    }

    pub(crate) fn acquire_avg_nanos(&self) -> u64 {
        self.acquire_total_nanos
            .checked_div(self.sampled_count)
            .unwrap_or(0)
    }

    fn percentile(hist: &Option<Histogram<u64>>, count: u64, p: f64) -> u64 {
        match hist {
            Some(hist) if count > 0 => hist.value_at_percentile(p.clamp(0.0, 100.0)),
            _ => 0,
        }
    }

    pub(crate) fn wait_percentile_nanos(&self, p: f64) -> u64 {
        Self::percentile(&self.wait_hist, self.sampled_count, p)
    }

    pub(crate) fn acquire_percentile_nanos(&self, p: f64) -> u64 {
        Self::percentile(&self.acquire_hist, self.sampled_count, p)
    }

    /// Bucket projections of the sampled wait/acquire durations for the
    /// Prometheus exporter (sparse native at `schema`, cumulative classic on
    /// `boundaries`).
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn native_wait_buckets(&self, schema: i32) -> Vec<(i32, u64)> {
        crate::lib_on::native_histograms::native_buckets_opt(
            self.wait_hist.as_ref(),
            self.sampled_count > 0,
            schema,
            crate::lib_on::native_histograms::NANOS_SCALE,
        )
    }

    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn classic_wait_buckets(&self, boundaries: &[u64]) -> Vec<u64> {
        crate::lib_on::native_histograms::classic_buckets_opt(
            self.wait_hist.as_ref(),
            self.sampled_count > 0,
            boundaries,
        )
    }

    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn native_acquire_buckets(&self, schema: i32) -> Vec<(i32, u64)> {
        crate::lib_on::native_histograms::native_buckets_opt(
            self.acquire_hist.as_ref(),
            self.sampled_count > 0,
            schema,
            crate::lib_on::native_histograms::NANOS_SCALE,
        )
    }

    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn classic_acquire_buckets(&self, boundaries: &[u64]) -> Vec<u64> {
        crate::lib_on::native_histograms::classic_buckets_opt(
            self.acquire_hist.as_ref(),
            self.sampled_count > 0,
            boundaries,
        )
    }
}

pub(crate) struct MutexesInternalState {
    pub(crate) stats: HashMap<u32, MutexEntry>,
}

pub(crate) struct MutexesState {
    pub(crate) inner: Arc<MetaRwLock<MutexesInternalState>>,
    pub(crate) shutdown_tx: StdMutex<Option<CbSender<()>>>,
    pub(crate) completion_rx: StdMutex<Option<CbReceiver<()>>>,
}

pub(crate) static MUTEXES_STATE: OnceLock<MutexesState> = OnceLock::new();

/// Runs `f` on the entries sorted for display, borrowed under the read lock:
/// nothing is cloned, so the per-entry histograms stay in the map.
pub(crate) fn with_sorted_mutex_entries<R>(f: impl FnOnce(&[&MutexEntry]) -> R) -> R {
    let Some(state) = MUTEXES_STATE.get() else {
        return f(&[]);
    };
    let guard = state.inner.read().unwrap();
    let mut stats: Vec<&MutexEntry> = guard.stats.values().collect();
    stats.sort_by(|a, b| compare_mutex_entries(a, b));
    f(&stats)
}

pub(crate) fn get_mutexes_json() -> crate::json::JsonMutexesList {
    let elapsed = std::time::Duration::from_nanos(crate::lib_on::current_elapsed_ns());
    let percentiles = crate::lib_on::hotpath_guard::configured_percentiles();
    with_sorted_mutex_entries(|entries| {
        crate::lib_on::report::collect_mutexes_json(entries, 0, elapsed, &percentiles, false)
    })
}

#[inline]
pub(crate) fn elapsed_nanos(start: Instant) -> u64 {
    start.elapsed().as_nanos() as u64
}

/// One sampling decision per acquisition; `None` skips both clock read pairs.
#[inline]
pub(crate) fn wait_stamp() -> Option<Instant> {
    crate::lib_on::sampling::mutexes_should_time().then(Instant::now)
}

static EVENT_QUEUES: EventQueueRegistry<MutexEvent> = EventQueueRegistry::new();

thread_local! {
    static EVENT_PRODUCER: EventProducer<MutexEvent> = EVENT_QUEUES.register();
}

#[inline]
pub(crate) fn send_mutex_event(event: MutexEvent) {
    if !EVENT_QUEUES.is_active() {
        return;
    }
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let _ = EVENT_PRODUCER.try_with(|producer| producer.push(event));
}

/// Stops producers ahead of the worker's final sweep at shutdown.
pub(crate) fn stop_mutex_events() {
    EVENT_QUEUES.set_active(false);
}

/// Entry for events that arrive ahead of their `Created` (sweeps only preserve
/// per-thread order, so another thread's data events can be drained first).
/// `Created` backfills the metadata.
fn placeholder_mutex_entry(id: u32) -> MutexEntry {
    MutexEntry {
        id,
        key: "",
        source: "",
        label: None,
        type_name: "",
        count: 0,
        sampled_count: 0,
        wait_total_nanos: 0,
        acquire_total_nanos: 0,
        wait_hist: Some(MutexEntry::new_histogram()),
        acquire_hist: Some(MutexEntry::new_histogram()),
        iter: 0,
    }
}

fn process_mutex_event(state: &mut MutexesInternalState, event: MutexEvent) {
    match event {
        MutexEvent::Created {
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
                .or_insert_with(|| placeholder_mutex_entry(id));
            entry.key = key;
            entry.source = source;
            entry.label = label;
            entry.type_name = type_name;
            entry.iter = iter;
        }
        MutexEvent::Released {
            id,
            wait_nanos,
            acquire_nanos,
        } => {
            let entry = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_mutex_entry(id));
            entry.count += 1;
            if let (Some(wait_nanos), Some(acquire_nanos)) = (wait_nanos, acquire_nanos) {
                entry.sampled_count += 1;
                entry.wait_total_nanos += wait_nanos;
                entry.acquire_total_nanos += acquire_nanos;
                MutexEntry::record(&mut entry.wait_hist, wait_nanos);
                MutexEntry::record(&mut entry.acquire_hist, acquire_nanos);
            }
        }
    }
}

/// Registers a new Mutex with the profiling subsystem.
pub(crate) fn register_mutex<T>(key: &'static str, label: Option<String>) -> u32 {
    let type_name = std::any::type_name::<T>();
    let source = crate::channels::display_source(key);
    init_mutexes_state();
    let id = next_mutex_id();

    send_mutex_event(MutexEvent::Created {
        id,
        key,
        source,
        label,
        type_name,
    });

    id
}

fn flush_mutex_buffer(buffer: &mut Vec<MutexEvent>, inner: &Arc<MetaRwLock<MutexesInternalState>>) {
    if buffer.is_empty() {
        return;
    }
    if let Ok(mut shared) = inner.write() {
        for e in buffer.drain(..) {
            process_mutex_event(&mut shared, e);
        }
    }
}

/// Initialize the lock statistics collection system (called on first instrumented lock).
pub(crate) fn init_mutexes_state() -> &'static MutexesState {
    MUTEXES_STATE.get_or_init(|| {
        START_TIME.get_or_init(Instant::now);

        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let (completion_tx, completion_rx) = bounded::<()>(1);

        let inner = Arc::new(meta_rw_lock!(
            "mutexes_state",
            MutexesInternalState {
                stats: HashMap::new(),
            },
        ));
        let inner_clone = Arc::clone(&inner);

        EVENT_QUEUES.set_active(true);

        crate::lib_on::spawn_profiler_thread("hp-mutexes", move || {
            let flush_interval = std::time::Duration::from_millis(*DRAIN_INTERVAL_MS);
            let mut swept: Vec<MutexEvent> = Vec::new();

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
                    flush_mutex_buffer(&mut swept, &inner_clone);
                    break;
                }

                EVENT_QUEUES.sweep(&mut swept);
                flush_mutex_buffer(&mut swept, &inner_clone);
            }

            let _ = completion_tx.send(());
        })
        .expect("Failed to spawn mutex-stats-collector thread");

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        MutexesState {
            inner,
            shutdown_tx: StdMutex::new(Some(shutdown_tx)),
            completion_rx: StdMutex::new(Some(completion_rx)),
        }
    })
}

/// Compare two lock stats for sorting. Custom labels first, then by source and iter.
pub(crate) fn compare_mutex_entries(a: &MutexEntry, b: &MutexEntry) -> std::cmp::Ordering {
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

/// Trait for instrumenting Mutexes. Dispatches on the type of the wrapped lock
/// (e.g. [`std::sync::Mutex`] or `parking_lot::Mutex`).
///
/// This trait is not intended for direct use. Use the `mutex!` macro instead.
#[doc(hidden)]
pub trait InstrumentMutex {
    type Output;
    fn instrument(self, source: &'static str, label: Option<String>) -> Self::Output;
}

/// Instrument an [`std::sync::Mutex`], `parking_lot::Mutex`, `tokio::sync::Mutex`, or
/// `async_lock::Mutex` for lock wait & acquire profiling.
///
/// Returns an instrumented drop-in replacement that proxies to the wrapped lock and records
/// how long the lock is waited for and held. The wrapper type matches the API of the underlying
/// lock (`std::sync::Mutex` returns `LockResult`s; `parking_lot::Mutex` returns guards directly;
/// `tokio::sync::Mutex` and `async_lock::Mutex` expose an async `lock` returning the guard
/// directly).
///
/// `parking_lot::Mutex` support requires the `parking_lot` feature; `tokio::sync::Mutex`
/// support requires the `tokio` feature; `async_lock::Mutex` support requires the `async-lock`
/// feature.
///
/// # Examples
///
/// ```rust,no_run
/// let lock = hotpath::mutex!(std::sync::Mutex::new(0u32));
/// *lock.lock().unwrap() += 1;
/// ```
#[macro_export]
macro_rules! mutex {
    ($expr:expr) => {{
        const MUTEX_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(MUTEX_ID);
        $crate::InstrumentMutex::instrument($expr, MUTEX_ID, None)
    }};

    // A literal label is checked for uniqueness, then re-dispatched wrapped in
    // a block so it takes the plain expression arm below.
    ($expr:expr, label = $label:literal) => {{
        $crate::__unique_label!(mutex, $label);
        $crate::mutex!($expr, label = { $label })
    }};

    ($expr:expr, label = $label:expr) => {{
        const MUTEX_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(MUTEX_ID);
        $crate::InstrumentMutex::instrument($expr, MUTEX_ID, Some($label.to_string()))
    }};
}
