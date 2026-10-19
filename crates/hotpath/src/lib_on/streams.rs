//! Stream instrumentation module - tracks items yielded and stream lifecycle.

use crossbeam_channel::{bounded, Receiver as CbReceiver, RecvTimeoutError, Sender as CbSender};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock as StdRwLock};

use crate::lib_on::{meta_rw_lock, MetaRwLock};

use crate::instant::Instant;

pub(crate) mod wrapper;

use crate::batch::{EventProducer, EventQueueRegistry};
use crate::channels::{resolve_label, LOGS_LIMIT};
use crate::json::JsonStreamEntry;
pub(crate) use crate::json::{ChannelState, DataFlowLogEntry, StreamLogs};
use crate::lib_on::hotpath_guard::DRAIN_INTERVAL_MS;
use crate::metrics_server::METRICS_SERVER_PORT;

static STREAM_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn next_stream_id() -> u32 {
    STREAM_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Entries are keyed by creation site and item type, so streams created
/// repeatedly at one `stream!` call (e.g. per handled request) share a single
/// accumulating entry and state stays bounded by the number of call sites
/// rather than the number of streams ever created. The site key includes the
/// column (`file:line:column`), so two invocations on one physical line do
/// not alias; the displayed source stays `file:line`.
type StreamSourceKey = (&'static str, &'static str);

static STREAM_SOURCE_IDS: OnceLock<StdRwLock<HashMap<StreamSourceKey, u32>>> = OnceLock::new();

/// Registers an instrumented stream. By default the entry id of earlier
/// streams from the same call site and item type is reused (only the first
/// registration emits [`StreamEvent::Created`], so its `label` wins); with
/// `iter` every instance gets its own entry, distinguished by the entry's
/// `iter` number. `Item` is the stream's item type, used to record the type
/// name and per-item byte size.
pub(crate) fn register_stream<Item>(key: &'static str, label: Option<String>, iter: bool) -> u32 {
    let type_name = std::any::type_name::<Item>();
    let source = crate::channels::display_source(key);
    init_streams_state();

    if !iter {
        let map = STREAM_SOURCE_IDS.get_or_init(|| StdRwLock::new(HashMap::new()));
        if let Some(&id) = map.read().unwrap().get(&(key, type_name)) {
            send_stream_event(StreamEvent::Instance { id });
            return id;
        }
        let mut writer = map.write().unwrap();
        if let Some(&id) = writer.get(&(key, type_name)) {
            send_stream_event(StreamEvent::Instance { id });
            return id;
        }
        let id = next_stream_id();
        writer.insert((key, type_name), id);

        send_stream_event(StreamEvent::Created {
            id,
            key,
            source,
            display_label: label,
            type_name,
            type_size: std::mem::size_of::<Item>(),
            iter_mode: false,
        });

        return id;
    }

    let id = next_stream_id();
    send_stream_event(StreamEvent::Created {
        id,
        key,
        source,
        display_label: label,
        type_name,
        type_size: std::mem::size_of::<Item>(),
        iter_mode: true,
    });

    id
}

/// Statistics for a single instrumented stream.
#[derive(Debug, Clone)]
pub(crate) struct StreamStats {
    pub(crate) id: u32,
    /// Column-including call-site key (`file:line:column`); the identity used
    /// by the display-suffix scan so same-line call sites do not cross-suffix.
    pub(crate) key: &'static str,
    /// The `file:line` form shown to users.
    pub(crate) source: &'static str,
    pub(crate) label: Option<String>,
    /// Number of stream instances aggregated into this entry.
    pub(crate) instances: u64,
    /// Number of aggregated instances whose `Completed` event was processed.
    pub(crate) closed_instances: u64,
    pub(crate) items_yielded: u64,
    pub(crate) type_name: &'static str,
    pub(crate) type_size: usize,
    pub(crate) iter: u32,
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl StreamStats {
    fn new(
        id: u32,
        key: &'static str,
        source: &'static str,
        label: Option<String>,
        type_name: &'static str,
        type_size: usize,
        iter: u32,
    ) -> Self {
        Self {
            id,
            key,
            source,
            label,
            instances: 0,
            closed_instances: 0,
            items_yielded: 0,
            type_name,
            type_size,
            iter,
        }
    }

    /// State shown to consumers: `None` for aggregated entries (`instances >
    /// 1`), whose instances complete independently - a single Active/Closed
    /// flag would flicker with churn and mislead. Single-instance and
    /// `iter = true` entries keep their exact state.
    pub(crate) fn display_state(&self) -> Option<ChannelState> {
        (self.instances <= 1).then(|| self.state())
    }

    /// Displayed state (only Active or Closed), derived from the instance
    /// counters. `>=` rather than `==` because a `Completed` event can be
    /// drained before the `Instance` event from another thread; the counters
    /// converge once idle.
    pub(crate) fn state(&self) -> ChannelState {
        if self.instances > 0 && self.closed_instances >= self.instances {
            ChannelState::Closed
        } else {
            ChannelState::Active
        }
    }
}

#[derive(Debug)]
pub(crate) struct StreamStatsLogs {
    pub(crate) logs: VecDeque<DataFlowLogEntry>,
}

impl StreamStatsLogs {
    fn new() -> Self {
        Self {
            logs: VecDeque::with_capacity(*LOGS_LIMIT),
        }
    }
}

pub(crate) struct StreamsInternalState {
    pub(crate) stats: HashMap<u32, StreamStats>,
    pub(crate) logs: HashMap<u32, StreamStatsLogs>,
}

impl From<&StreamStats> for JsonStreamEntry {
    fn from(stats: &StreamStats) -> Self {
        let label = resolve_label(stats.source, stats.label.as_deref(), Some(stats.iter));

        JsonStreamEntry {
            id: stats.id,
            source: stats.source.to_string(),
            label,
            has_custom_label: stats.label.is_some(),
            state: stats.display_state().map(|s| s.as_str().to_string()),
            instances: stats.instances,
            closed_instances: stats.closed_instances,
            items_yielded: stats.items_yielded,
            type_name: stats.type_name.to_string(),
            type_size: stats.type_size,
            location: crate::lib_on::locations::location_for_key(stats.key),
            iter: stats.iter,
        }
    }
}

/// Events sent to the background stream statistics collection thread.
#[derive(Debug)]
pub(crate) enum StreamEvent {
    Created {
        id: u32,
        /// Column-including call-site key (`file:line:column`); distinguishes
        /// same-line invocations in the display-suffix scan. `source` is the
        /// `file:line` shown to users.
        key: &'static str,
        source: &'static str,
        display_label: Option<String>,
        type_name: &'static str,
        type_size: usize,
        /// `true` when the registration opted into per-instance entries
        /// (`iter = true`); gates the display-suffix scan.
        iter_mode: bool,
    },
    /// A repeat default-mode registration at an already-known call site. The
    /// first instance is counted by `Created`; each later one sends this
    /// instead.
    Instance {
        id: u32,
    },
    Yielded {
        id: u32,
        log: Option<String>,
        timestamp: Instant,
    },
    Completed {
        id: u32,
    },
}

pub(crate) struct StreamsState {
    pub(crate) inner: Arc<MetaRwLock<StreamsInternalState>>,
    pub(crate) shutdown_tx: Mutex<Option<CbSender<()>>>,
    pub(crate) completion_rx: Mutex<Option<CbReceiver<()>>>,
}

pub(crate) static STREAMS_STATE: OnceLock<StreamsState> = OnceLock::new();

static EVENT_QUEUES: EventQueueRegistry<StreamEvent> = EventQueueRegistry::new();

thread_local! {
    static EVENT_PRODUCER: EventProducer<StreamEvent> = EVENT_QUEUES.register();
}

#[inline]
pub(crate) fn send_stream_event(event: StreamEvent) {
    if !EVENT_QUEUES.is_active() {
        return;
    }
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let _ = EVENT_PRODUCER.try_with(|producer| producer.push(event));
}

/// Stops producers ahead of the worker's final sweep at shutdown.
pub(crate) fn stop_stream_events() {
    EVENT_QUEUES.set_active(false);
}

/// Entry for events that arrive ahead of their `Created` (sweeps only preserve
/// per-thread order, so another thread's data events can be drained first).
/// `Created` backfills the metadata.
fn placeholder_stream_stats(id: u32) -> StreamStats {
    StreamStats::new(id, "", "", None, "", 0, 0)
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn process_stream_event(state: &mut StreamsInternalState, event: StreamEvent) {
    match event {
        StreamEvent::Created {
            id,
            key,
            source,
            display_label,
            type_name,
            type_size,
            iter_mode,
        } => {
            // The O(n) same-site scan only has meaning for per-instance
            // entries; default-mode entries are unique per site key and keep
            // an unsuffixed label.
            let iter = if iter_mode {
                state.stats.values().filter(|s| s.key == key).count() as u32
            } else {
                0
            };
            let entry = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_stream_stats(id));
            entry.key = key;
            entry.source = source;
            entry.label = display_label;
            entry.type_name = type_name;
            entry.type_size = type_size;
            entry.iter = iter;
            entry.instances += 1;
            state.logs.entry(id).or_insert_with(StreamStatsLogs::new);
        }
        StreamEvent::Instance { id } => {
            state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_stream_stats(id))
                .instances += 1;
        }
        StreamEvent::Yielded { id, log, timestamp } => {
            let stream_stats = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_stream_stats(id));
            stream_stats.items_yielded += 1;
            let items_yielded = stream_stats.items_yielded;

            let entry_logs = state.logs.entry(id).or_insert_with(StreamStatsLogs::new);
            let limit = *crate::channels::LOGS_LIMIT;
            if entry_logs.logs.len() >= limit {
                entry_logs.logs.pop_front();
            }
            entry_logs.logs.push_back(DataFlowLogEntry::new(
                items_yielded,
                crate::channels::timestamp_nanos(timestamp),
                log,
                None,
                None,
                None,
            ));
        }
        StreamEvent::Completed { id } => {
            state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_stream_stats(id))
                .closed_instances += 1;
        }
    }
}

fn flush_stream_buffer(
    buffer: &mut Vec<StreamEvent>,
    inner: &Arc<MetaRwLock<StreamsInternalState>>,
) {
    if buffer.is_empty() {
        return;
    }
    if let Ok(mut shared) = inner.write() {
        for e in buffer.drain(..) {
            process_stream_event(&mut shared, e);
        }
    }
}

/// Initialize the stream statistics collection system (called on first instrumented stream).
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
pub(crate) fn init_streams_state() -> &'static StreamsState {
    STREAMS_STATE.get_or_init(|| {
        crate::lib_on::START_TIME.get_or_init(Instant::now);

        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let (completion_tx, completion_rx) = bounded::<()>(1);
        let inner = Arc::new(meta_rw_lock!(
            "streams_state",
            StreamsInternalState {
                stats: HashMap::new(),
                logs: HashMap::new(),
            },
        ));
        let inner_clone = Arc::clone(&inner);

        EVENT_QUEUES.set_active(true);

        crate::lib_on::spawn_profiler_thread("hp-streams", move || {
            let flush_interval = std::time::Duration::from_millis(*DRAIN_INTERVAL_MS);
            let mut swept: Vec<StreamEvent> = Vec::new();

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
                    flush_stream_buffer(&mut swept, &inner_clone);
                    break;
                }

                EVENT_QUEUES.sweep(&mut swept);
                flush_stream_buffer(&mut swept, &inner_clone);
            }

            let _ = completion_tx.send(());
        })
        .expect("Failed to spawn stream-stats-collector thread");

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        StreamsState {
            inner,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            completion_rx: Mutex::new(Some(completion_rx)),
        }
    })
}

/// Trait for instrumenting streams.
///
/// This trait is not intended for direct use. Use the `stream!` macro instead.
#[doc(hidden)]
pub trait InstrumentStream {
    type Output;
    fn instrument_stream(
        self,
        source: &'static str,
        label: Option<String>,
        iter: bool,
    ) -> Self::Output;
}

/// Trait for instrumenting streams with message logging.
///
/// This trait is not intended for direct use. Use the `stream!` macro with `log = true` instead.
#[doc(hidden)]
pub trait InstrumentStreamLog {
    type Output;
    fn instrument_stream_log(
        self,
        source: &'static str,
        label: Option<String>,
        iter: bool,
    ) -> Self::Output;
}

impl<S> InstrumentStream for S
where
    S: futures_core::Stream,
{
    type Output = crate::streams::wrapper::InstrumentedStream<S>;

    fn instrument_stream(
        self,
        source: &'static str,
        label: Option<String>,
        iter: bool,
    ) -> Self::Output {
        crate::streams::wrapper::InstrumentedStream::new(self, source, label, iter)
    }
}

impl<S> InstrumentStreamLog for S
where
    S: futures_core::Stream,
    S::Item: std::fmt::Debug,
{
    type Output = crate::streams::wrapper::InstrumentedStreamLog<S>;

    fn instrument_stream_log(
        self,
        source: &'static str,
        label: Option<String>,
        iter: bool,
    ) -> Self::Output {
        crate::streams::wrapper::InstrumentedStreamLog::new(self, source, label, iter)
    }
}

/// Instrument a stream to track its item yields.
///
/// Optional parameters: `label`, `log = true`, `iter = true` (in any order).
/// `log = true` requires `Debug` on the item type.
///
/// # Call-site aggregation and `iter = true`
///
/// By default all streams created at one `stream!` call site (with the same
/// item type) accumulate into a **single entry**: `items_yielded` is summed
/// across instances and the `Inst` column reports how many instances the
/// entry aggregates, so profiler state stays bounded even when a call site
/// creates a stream per request. The first registration's `label` wins.
/// Aggregated entries show `-` for state (their instances complete
/// independently). Log windows (`log = true`) interleave items from all
/// instances.
///
/// Pass `iter = true` to give every instance its own entry instead (displayed
/// as `label`, `label-2`, `label-3`, ...). Profiler state then grows with the
/// number of streams ever created, so prefer the default aggregation for call
/// sites with unbounded instance churn.
///
/// # Examples
///
/// ```rust,ignore
/// use futures::stream::{self, StreamExt};
/// use hotpath::stream;
///
/// #[tokio::main]
/// async fn main() {
///     let s = stream!(stream::iter(1..=10));
///     let _items: Vec<_> = s.collect().await;
/// }
/// ```
#[macro_export]
macro_rules! stream {
    ($expr:expr) => {{
        const STREAM_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(STREAM_ID);
        $crate::InstrumentStream::instrument_stream($expr, STREAM_ID, None, false)
    }};

    // Any argument list is parsed order-independently by the muncher below.
    // Slots are `label log iter`; `label`/`iter` are stored as ready-to-use
    // expression tokens so the dispatch only branches on `log`. `STREAM_ID` is
    // captured once here so `file!()`/`line!()` resolve to the user's call site.
    ($expr:expr, $($rest:tt)*) => {{
        const STREAM_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(STREAM_ID);
        $crate::stream!(@munch STREAM_ID, $expr ; (None) [nolog] (false) ; $($rest)*)
    }};

    (@munch $id:ident, $e:expr ; $lbl:tt $log:tt $it:tt ;) => {
        $crate::stream!(@dispatch $id, $e ; $lbl $log $it)
    };
    // A literal label is checked for uniqueness, then re-munched wrapped in a
    // block so it takes the plain expression arm below.
    (@munch $id:ident, $e:expr ; $lbl:tt $log:tt $it:tt ; label = $l:literal $(, $($r:tt)*)?) => {{
        $crate::__unique_label!(stream, $l);
        $crate::stream!(@munch $id, $e ; $lbl $log $it ; label = { $l } $(, $($r)*)?)
    }};
    (@munch $id:ident, $e:expr ; $lbl:tt $log:tt $it:tt ; label = $l:expr $(, $($r:tt)*)?) => {
        $crate::stream!(@munch $id, $e ; (Some($l.to_string())) $log $it ; $($($r)*)?)
    };
    (@munch $id:ident, $e:expr ; $lbl:tt $log:tt $it:tt ; log = true $(, $($r:tt)*)?) => {
        $crate::stream!(@munch $id, $e ; $lbl [log] $it ; $($($r)*)?)
    };
    (@munch $id:ident, $e:expr ; $lbl:tt $log:tt $it:tt ; iter = true $(, $($r:tt)*)?) => {
        $crate::stream!(@munch $id, $e ; $lbl $log (true) ; $($($r)*)?)
    };

    (@dispatch $id:ident, $e:expr ; $lbl:tt [nolog] $it:tt) => {
        $crate::InstrumentStream::instrument_stream($e, $id, $lbl, $it)
    };
    (@dispatch $id:ident, $e:expr ; $lbl:tt [log] $it:tt) => {
        $crate::InstrumentStreamLog::instrument_stream_log($e, $id, $lbl, $it)
    };
}

/// Custom labels come first (sorted alphabetically), then auto-generated labels (sorted by source and iter).
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn compare_stream_stats(a: &StreamStats, b: &StreamStats) -> std::cmp::Ordering {
    let a_has_label = a.label.is_some();
    let b_has_label = b.label.is_some();

    match (a_has_label, b_has_label) {
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

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_sorted_stream_stats() -> Vec<StreamStats> {
    let Some(state) = STREAMS_STATE.get() else {
        return Vec::new();
    };
    let guard = state.inner.read().unwrap();
    let mut stats: Vec<StreamStats> = guard.stats.values().cloned().collect();
    stats.sort_by(compare_stream_stats);
    stats
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_streams_json() -> crate::json::JsonStreamsList {
    let data: Vec<JsonStreamEntry> = get_sorted_stream_stats()
        .iter()
        .map(JsonStreamEntry::from)
        .collect();

    crate::json::JsonStreamsList {
        current_elapsed_ns: crate::lib_on::current_elapsed_ns(),
        total_count: data.len(),
        included_count: data.len(),
        data,
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_stream_logs(id: u32) -> Option<StreamLogs> {
    let state = STREAMS_STATE.get()?;
    let guard = state.inner.read().unwrap();
    let entry_logs = guard.logs.get(&id)?;
    let mut yielded_logs: Vec<DataFlowLogEntry> = entry_logs.logs.iter().cloned().collect();
    yielded_logs.sort_by_key(|entry| std::cmp::Reverse(entry.index));
    Some(StreamLogs {
        id,
        logs: yielded_logs,
    })
}
