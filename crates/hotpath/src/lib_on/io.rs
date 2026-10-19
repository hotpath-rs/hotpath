//! Byte-level I/O instrumentation - tracks read/write/flush/shutdown operations
//! performed through wrapped `Read`/`Write`/`AsyncRead`/`AsyncWrite` values.
//!
//! Wrapping the underlying resource (file, socket) measures actual resource
//! I/O; wrapping a `BufReader`/`BufWriter` measures application-facing
//! buffered operations instead.

use crossbeam_channel::{bounded, Receiver as CbReceiver, RecvTimeoutError, Sender as CbSender};
use hdrhistogram::Histogram;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, RwLock as StdRwLock};

use crate::batch::{EventProducer, EventQueueRegistry};
use crate::instant::Instant;
use crate::lib_on::hotpath_guard::DRAIN_INTERVAL_MS;
use crate::lib_on::START_TIME;
use crate::metrics_server::METRICS_SERVER_PORT;

pub(crate) mod wrapper;

pub use wrapper::{io_unwrap, InstrumentedIo};

static IO_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

fn next_io_id() -> u32 {
    IO_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Kind of I/O operation performed through an instrumented wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IoOpKind {
    Read,
    Write,
    Flush,
    #[cfg_attr(not(feature = "tokio"), allow(dead_code))]
    Shutdown,
}

/// Events sent to the background I/O statistics collection thread.
#[derive(Debug)]
pub(crate) enum IoEvent {
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
    /// A completed operation. For async I/O the duration spans from the first
    /// poll of the operation to `Ready`, so it includes suspended time.
    /// `duration_nanos` is `None` when timing was skipped under time sampling;
    /// the event still counts the operation and its bytes.
    Op {
        id: u32,
        kind: IoOpKind,
        bytes: u64,
        duration_nanos: Option<u64>,
    },
    /// An operation that failed. Retryable conditions (`WouldBlock`,
    /// `Interrupted`) are not reported as errors.
    Error { id: u32, kind: IoOpKind },
    /// A repeat wrapper registration at an already-known call site. The first
    /// instance is counted by `Created`; each later one sends this instead.
    Instance { id: u32 },
}

/// Per-operation-kind statistics for a single instrumented I/O value.
#[derive(Debug, Clone)]
pub(crate) struct IoOpStats {
    pub(crate) count: u64,
    pub(crate) sampled_count: u64,
    pub(crate) bytes: u64,
    /// Bytes from timed operations only; pairs with `total_nanos` so
    /// throughput stays correct under time sampling.
    pub(crate) sampled_bytes: u64,
    pub(crate) errors: u64,
    pub(crate) total_nanos: u64,
    hist: Option<Histogram<u64>>,
}

impl IoOpStats {
    const LOW_NS: u64 = 1;
    const HIGH_NS: u64 = crate::lib_on::MAX_DURATION_NS;
    const SIGFIGS: u8 = 3;

    fn new() -> Self {
        Self {
            count: 0,
            sampled_count: 0,
            bytes: 0,
            sampled_bytes: 0,
            errors: 0,
            total_nanos: 0,
            hist: Some(
                Histogram::<u64>::new_with_bounds(Self::LOW_NS, Self::HIGH_NS, Self::SIGFIGS)
                    .expect("hdrhistogram init"),
            ),
        }
    }

    fn record(&mut self, bytes: u64, duration_nanos: Option<u64>) {
        self.count += 1;
        self.bytes += bytes;
        if let Some(nanos) = duration_nanos {
            self.sampled_count += 1;
            self.sampled_bytes += bytes;
            self.total_nanos += nanos;
            if let Some(ref mut hist) = self.hist {
                hist.record(nanos.clamp(Self::LOW_NS, Self::HIGH_NS))
                    .unwrap();
            }
        }
    }

    pub(crate) fn avg_nanos(&self) -> u64 {
        self.total_nanos
            .checked_div(self.sampled_count)
            .unwrap_or(0)
    }

    /// Estimated transfer rate while operations were in flight, based on
    /// timed operations only. `None` when nothing was timed.
    pub(crate) fn throughput_bytes_per_sec(&self) -> Option<f64> {
        if self.total_nanos == 0 {
            return None;
        }
        Some(self.sampled_bytes as f64 * 1_000_000_000.0 / self.total_nanos as f64)
    }

    pub(crate) fn percentile_nanos(&self, p: f64) -> u64 {
        match &self.hist {
            Some(hist) if self.sampled_count > 0 => hist.value_at_percentile(p.clamp(0.0, 100.0)),
            _ => 0,
        }
    }

    /// Bucket projections of the sampled op durations for the Prometheus
    /// exporter (sparse native at `schema`, cumulative classic on
    /// `boundaries`).
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn native_buckets(&self, schema: i32) -> Vec<(i32, u64)> {
        crate::lib_on::native_histograms::native_buckets_opt(
            self.hist.as_ref(),
            self.sampled_count > 0,
            schema,
            crate::lib_on::native_histograms::NANOS_SCALE,
        )
    }

    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn classic_buckets(&self, boundaries: &[u64]) -> Vec<u64> {
        crate::lib_on::native_histograms::classic_buckets_opt(
            self.hist.as_ref(),
            self.sampled_count > 0,
            boundaries,
        )
    }
}

/// Statistics for a single `io!` creation site (source location + concrete
/// type). All wrapper instances from that site accumulate into one entry.
#[derive(Debug)]
pub(crate) struct IoEntry {
    pub(crate) id: u32,
    /// Column-including call-site key (`file:line:column`); the identity used
    /// by the display-suffix scan so same-line call sites do not cross-suffix.
    pub(crate) key: &'static str,
    /// The `file:line` form shown to users.
    pub(crate) source: &'static str,
    pub(crate) label: Option<String>,
    pub(crate) type_name: &'static str,
    pub(crate) read: IoOpStats,
    pub(crate) write: IoOpStats,
    pub(crate) flush: IoOpStats,
    pub(crate) shutdown: IoOpStats,
    /// Number of wrapper instances aggregated into this entry.
    pub(crate) instances: u32,
    pub(crate) iter: u32,
}

impl IoEntry {
    pub(crate) fn op(&self, kind: IoOpKind) -> &IoOpStats {
        match kind {
            IoOpKind::Read => &self.read,
            IoOpKind::Write => &self.write,
            IoOpKind::Flush => &self.flush,
            IoOpKind::Shutdown => &self.shutdown,
        }
    }

    fn op_mut(&mut self, kind: IoOpKind) -> &mut IoOpStats {
        match kind {
            IoOpKind::Read => &mut self.read,
            IoOpKind::Write => &mut self.write,
            IoOpKind::Flush => &mut self.flush,
            IoOpKind::Shutdown => &mut self.shutdown,
        }
    }

    /// Errors across all write-side kinds, shown on the write sub-table row so
    /// flush failures (where buffered writers surface deferred errors) aren't
    /// hidden from the table.
    pub(crate) fn write_side_errors(&self) -> u64 {
        self.write.errors + self.flush.errors + self.shutdown.errors
    }
}

pub(crate) struct IoInternalState {
    pub(crate) stats: HashMap<u32, IoEntry>,
}

pub(crate) struct IoState {
    pub(crate) inner: Arc<StdRwLock<IoInternalState>>,
    pub(crate) shutdown_tx: StdMutex<Option<CbSender<()>>>,
    pub(crate) completion_rx: StdMutex<Option<CbReceiver<()>>>,
}

pub(crate) static IO_STATE: OnceLock<IoState> = OnceLock::new();

/// Runs `f` on the entries sorted for display, borrowed under the read lock:
/// nothing is cloned, so the per-entry histograms stay in the map.
pub(crate) fn with_sorted_io_entries<R>(f: impl FnOnce(&[&IoEntry]) -> R) -> R {
    let Some(state) = IO_STATE.get() else {
        return f(&[]);
    };
    let guard = state.inner.read().unwrap();
    let mut stats: Vec<&IoEntry> = guard.stats.values().collect();
    stats.sort_by(|a, b| compare_io_entries(a, b));
    f(&stats)
}

pub(crate) fn get_io_json() -> crate::json::JsonIoList {
    let elapsed = std::time::Duration::from_nanos(crate::lib_on::current_elapsed_ns());
    let percentiles = crate::lib_on::hotpath_guard::configured_percentiles();
    with_sorted_io_entries(|entries| {
        crate::lib_on::report::collect_io_json(entries, 0, elapsed, &percentiles, false)
    })
}

#[inline]
pub(crate) fn elapsed_nanos(start: Instant) -> u64 {
    start.elapsed().as_nanos() as u64
}

/// One sampling decision per operation; `None` skips both clock reads.
#[inline]
pub(crate) fn op_stamp() -> Option<Instant> {
    crate::lib_on::sampling::io_should_time().then(Instant::now)
}

static EVENT_QUEUES: EventQueueRegistry<IoEvent> = EventQueueRegistry::new();

thread_local! {
    static EVENT_PRODUCER: EventProducer<IoEvent> = EVENT_QUEUES.register();
}

#[inline]
pub(crate) fn send_io_event(event: IoEvent) {
    if !EVENT_QUEUES.is_active() {
        return;
    }
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let _ = EVENT_PRODUCER.try_with(|producer| producer.push(event));
}

/// Stops producers ahead of the worker's final sweep at shutdown.
pub(crate) fn stop_io_events() {
    EVENT_QUEUES.set_active(false);
}

/// Entry for events that arrive ahead of their `Created` (sweeps only preserve
/// per-thread order, so another thread's data events can be drained first).
/// `Created` backfills the metadata.
fn placeholder_io_entry(id: u32) -> IoEntry {
    IoEntry {
        id,
        key: "",
        source: "",
        label: None,
        type_name: "",
        read: IoOpStats::new(),
        write: IoOpStats::new(),
        flush: IoOpStats::new(),
        shutdown: IoOpStats::new(),
        instances: 0,
        iter: 0,
    }
}

fn process_io_event(state: &mut IoInternalState, event: IoEvent) {
    match event {
        IoEvent::Created {
            id,
            key,
            source,
            label,
            type_name,
        } => {
            let iter = state
                .stats
                .values()
                .filter(|s| s.key == key && s.label == label)
                .count() as u32;
            let entry = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_io_entry(id));
            entry.key = key;
            entry.source = source;
            entry.label = label;
            entry.type_name = type_name;
            entry.instances += 1;
            entry.iter = iter;
        }
        IoEvent::Op {
            id,
            kind,
            bytes,
            duration_nanos,
        } => {
            let entry = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_io_entry(id));
            entry.op_mut(kind).record(bytes, duration_nanos);
        }
        IoEvent::Error { id, kind } => {
            let entry = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_io_entry(id));
            entry.op_mut(kind).errors += 1;
        }
        IoEvent::Instance { id } => {
            let entry = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_io_entry(id));
            entry.instances += 1;
        }
    }
}

/// `(site key, type)`: wrappers created repeatedly at one `io!` call (e.g.
/// per accepted connection) share one entry, so state stays bounded by call
/// sites. The site key includes the column so same-line invocations do not
/// alias.
type IoSourceKey = (&'static str, &'static str);

static IO_SOURCE_IDS: OnceLock<StdRwLock<HashMap<IoSourceKey, u32>>> = OnceLock::new();

/// Registers an instrumented I/O value. By default the entry id of earlier
/// wrappers from the same call site and type is reused (only the first
/// registration emits `Created`, so its `label` wins); with `iter` every
/// instance gets its own entry. Instances sharing a source and label are
/// distinguished by the entry's `iter` number; a distinct label per instance
/// keeps each entry's label as provided.
pub(crate) fn register_io<T>(key: &'static str, label: Option<String>, iter: bool) -> u32 {
    let type_name = std::any::type_name::<T>();
    let source = crate::channels::display_source(key);
    init_io_state();

    if !iter {
        let map = IO_SOURCE_IDS.get_or_init(|| StdRwLock::new(HashMap::new()));
        if let Some(&id) = map.read().unwrap().get(&(key, type_name)) {
            send_io_event(IoEvent::Instance { id });
            return id;
        }
        let mut writer = map.write().unwrap();
        if let Some(&id) = writer.get(&(key, type_name)) {
            send_io_event(IoEvent::Instance { id });
            return id;
        }
        let id = next_io_id();
        writer.insert((key, type_name), id);

        send_io_event(IoEvent::Created {
            id,
            key,
            source,
            label,
            type_name,
        });

        return id;
    }

    let id = next_io_id();
    send_io_event(IoEvent::Created {
        id,
        key,
        source,
        label,
        type_name,
    });

    id
}

fn flush_io_buffer(buffer: &mut Vec<IoEvent>, inner: &Arc<StdRwLock<IoInternalState>>) {
    if buffer.is_empty() {
        return;
    }
    if let Ok(mut shared) = inner.write() {
        for e in buffer.drain(..) {
            process_io_event(&mut shared, e);
        }
    }
}

/// Initialize the I/O statistics collection system (called on first instrumented value).
pub(crate) fn init_io_state() -> &'static IoState {
    IO_STATE.get_or_init(|| {
        START_TIME.get_or_init(Instant::now);

        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let (completion_tx, completion_rx) = bounded::<()>(1);

        let inner = Arc::new(StdRwLock::new(IoInternalState {
            stats: HashMap::new(),
        }));
        let inner_clone = Arc::clone(&inner);

        EVENT_QUEUES.set_active(true);

        crate::lib_on::spawn_profiler_thread("hp-io", move || {
            let flush_interval = std::time::Duration::from_millis(*DRAIN_INTERVAL_MS);
            let mut swept: Vec<IoEvent> = Vec::new();

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
                    flush_io_buffer(&mut swept, &inner_clone);
                    break;
                }

                EVENT_QUEUES.sweep(&mut swept);
                flush_io_buffer(&mut swept, &inner_clone);
            }

            let _ = completion_tx.send(());
        })
        .expect("Failed to spawn io-stats-collector thread");

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        IoState {
            inner,
            shutdown_tx: StdMutex::new(Some(shutdown_tx)),
            completion_rx: StdMutex::new(Some(completion_rx)),
        }
    })
}

/// Compare two I/O stats for sorting. Custom labels first, then by source and iter.
pub(crate) fn compare_io_entries(a: &IoEntry, b: &IoEntry) -> std::cmp::Ordering {
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

/// Instrument a value implementing [`std::io::Read`], [`std::io::Write`],
/// `tokio::io::AsyncRead`, or `tokio::io::AsyncWrite` for byte-level I/O
/// profiling (`tokio` traits require the `tokio` feature).
///
/// Returns an [`InstrumentedIo`] wrapper that delegates every operation to the
/// wrapped value and records operation counts, bytes processed, durations, and
/// errors. Synchronous reads and writes measure the full method call; async
/// operations measure from the first poll to `Ready`, so reported durations
/// include async waiting time.
///
/// The reported `Rate` is per-operation transfer speed: bytes divided by
/// summed in-flight operation time. When one call site is shared by wrappers
/// operating concurrently (e.g. per accepted connection), the entry
/// aggregates their operations and the rate stays duration-weighted per
/// operation - it is not the call site's aggregate bandwidth. The `Inst`
/// column reports how many wrapper instances the entry aggregates, so
/// `Rate * Inst` bounds the call site's aggregate bandwidth from above when
/// all instances operate concurrently. See
/// `test-io/examples/concurrent_io.rs`.
///
/// Pass `iter = true` to give every wrapper instance its own entry instead,
/// e.g. one row per accepted connection with its individual rate and byte
/// counts. Instances sharing one label are displayed as `label`, `label-2`,
/// `label-3`, ...; a dynamic `label` expression that differs per instance
/// (e.g. `format!("conn-{i}")`) names each row as provided. Profiler
/// state then grows with the number of instances ever created, so prefer the
/// default aggregation for call sites with unbounded instance churn.
///
/// Wrapping the underlying resource (file, socket) measures actual resource
/// I/O; wrapping a `BufReader`/`BufWriter` measures application-facing
/// buffered operations.
///
/// The wrapper derefs to the wrapped value; for consuming methods of the
/// wrapped type (e.g. a codec's `finish(self)`) unwrap first with
/// [`io_unwrap`](crate::io_unwrap).
///
/// # Examples
///
/// ```rust,no_run
/// use std::io::Read;
///
/// let mut reader = hotpath::io!(std::io::Cursor::new(vec![1u8, 2, 3]), label = "cursor");
/// let mut buf = [0u8; 3];
/// reader.read_exact(&mut buf).unwrap();
/// ```
#[macro_export]
macro_rules! io {
    ($expr:expr) => {{
        const IO_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(IO_ID);
        $crate::io::InstrumentedIo::__new_instrumented($expr, IO_ID, None, false)
    }};

    // Literal labels are checked for uniqueness, then re-dispatched wrapped in
    // a block so they take the plain expression arms below.
    ($expr:expr, label = $label:literal) => {{
        $crate::__unique_label!(io, $label);
        $crate::io!($expr, label = { $label })
    }};
    ($expr:expr, label = $label:literal, iter = true) => {{
        $crate::__unique_label!(io, $label);
        $crate::io!($expr, label = { $label }, iter = true)
    }};
    ($expr:expr, iter = true, label = $label:literal) => {{
        $crate::__unique_label!(io, $label);
        $crate::io!($expr, iter = true, label = { $label })
    }};

    ($expr:expr, label = $label:expr) => {{
        const IO_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(IO_ID);
        $crate::io::InstrumentedIo::__new_instrumented(
            $expr,
            IO_ID,
            Some($label.to_string()),
            false,
        )
    }};

    ($expr:expr, iter = true) => {{
        const IO_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(IO_ID);
        $crate::io::InstrumentedIo::__new_instrumented($expr, IO_ID, None, true)
    }};

    ($expr:expr, label = $label:expr, iter = true) => {{
        const IO_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(IO_ID);
        $crate::io::InstrumentedIo::__new_instrumented($expr, IO_ID, Some($label.to_string()), true)
    }};

    ($expr:expr, iter = true, label = $label:expr) => {{
        const IO_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(IO_ID);
        $crate::io::InstrumentedIo::__new_instrumented($expr, IO_ID, Some($label.to_string()), true)
    }};
}
