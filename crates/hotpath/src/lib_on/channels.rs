//! Channel instrumentation module - tracks message flow and channel state.

use crossbeam_channel::{bounded, Receiver as CbReceiver, RecvTimeoutError, Sender as CbSender};
use hdrhistogram::Histogram;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock as StdRwLock};

use crate::lib_on::{meta_rw_lock, MetaRwLock};

use crate::instant::Instant;

pub(crate) mod wrapper;

use crate::channels::wrapper::WrapChannel;

use std::mem;

use crate::batch::{EventProducer, EventQueueRegistry};
use crate::json::JsonChannelEntry;
pub(crate) use crate::json::{ChannelLogs, ChannelState, DataFlowLogEntry};
use crate::lib_on::hotpath_guard::DRAIN_INTERVAL_MS;
use crate::metrics_server::METRICS_SERVER_PORT;

static CHANNEL_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn next_channel_id() -> u32 {
    CHANNEL_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

pub(crate) use crate::json::ChannelType;

/// Entries are keyed by creation site and message type, so channels created
/// repeatedly at one `channel!` call (e.g. per handled request) share a single
/// accumulating entry and state stays bounded by the number of call sites
/// rather than the number of channels ever created. The site key includes the
/// column (`file:line:column`), so two invocations on one physical line do
/// not alias; the displayed source stays `file:line`.
type ChannelSourceKey = (&'static str, &'static str);

static CHANNEL_SOURCE_IDS: OnceLock<StdRwLock<HashMap<ChannelSourceKey, u32>>> = OnceLock::new();

/// Registers a new channel with the profiling subsystem.
///
/// By default the entry id of earlier channels from the same call site and
/// message type is reused (only the first registration emits
/// [`ChannelEvent::Created`], so its `label` and `channel_type` win); with
/// `iter` every instance gets its own entry, distinguished by the entry's
/// `iter` number. Returns the channel's id, which wrappers use to report
/// subsequent send/receive/close events. `T` is the message type carried by
/// the channel and is used to record the type name and per-message byte size.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
pub(crate) fn register_channel<T>(
    key: &'static str,
    label: Option<String>,
    channel_type: ChannelType,
    iter: bool,
) -> u32 {
    let type_name = std::any::type_name::<T>();
    let source = display_source(key);
    init_channels_state();

    if !iter {
        let map = CHANNEL_SOURCE_IDS.get_or_init(|| StdRwLock::new(HashMap::new()));
        if let Some(&id) = map.read().unwrap().get(&(key, type_name)) {
            send_channel_event(ChannelEvent::Instance { id });
            return id;
        }
        let mut writer = map.write().unwrap();
        if let Some(&id) = writer.get(&(key, type_name)) {
            send_channel_event(ChannelEvent::Instance { id });
            return id;
        }
        let id = next_channel_id();
        writer.insert((key, type_name), id);

        send_channel_event(ChannelEvent::Created {
            id,
            key,
            source,
            display_label: label,
            channel_type,
            type_name,
            type_size: mem::size_of::<T>(),
            iter_mode: false,
        });

        return id;
    }

    let id = next_channel_id();
    send_channel_event(ChannelEvent::Created {
        id,
        key,
        source,
        display_label: label,
        channel_type,
        type_name,
        type_size: mem::size_of::<T>(),
        iter_mode: true,
    });

    id
}

static EVENT_QUEUES: EventQueueRegistry<ChannelEvent> = EventQueueRegistry::new();

thread_local! {
    static EVENT_PRODUCER: EventProducer<ChannelEvent> = EVENT_QUEUES.register();
}

#[inline]
pub(crate) fn send_channel_event(event: ChannelEvent) {
    if !EVENT_QUEUES.is_active() {
        return;
    }
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    // `try_with`, not `with`: an endpoint can emit an event (send,
    // recv, or `Closed` on drop) from a producer thread that is tearing down, when
    // this thread-local may already be destroyed. Dropping the event is fine;
    // panicking in a `Drop` would abort the process.
    let _ = EVENT_PRODUCER.try_with(|producer| producer.push(event));
}

/// Stops producers ahead of the worker's final sweep at shutdown.
pub(crate) fn stop_channel_events() {
    EVENT_QUEUES.set_active(false);
}

/// Emits [`ChannelEvent::Closed`] exactly once per channel instance. Both
/// endpoints of a wrap channel can close independently (last sender dropped,
/// receiver dropped); the first one wins so `closed_instances` counts
/// instances, not endpoints. `closed` is the flag shared by the instance's
/// endpoint wrappers.
///
/// `remaining` is the caller's count of messages still queued in the
/// instance. The second endpoint to close fully tears the instance down, so
/// its `remaining` messages can never be received; they are reported as
/// [`ChannelEvent::Abandoned`] so the aggregated entry's counts-derived queue
/// depth does not carry the sent-minus-received deficit forever.
pub(crate) fn mark_closed(closed: &std::sync::atomic::AtomicBool, id: u32, remaining: usize) {
    if !closed.swap(true, Ordering::AcqRel) {
        send_channel_event(ChannelEvent::Closed { id });
    } else if remaining > 0 {
        send_channel_event(ChannelEvent::Abandoned {
            id,
            count: remaining as u64,
        });
    }
}

/// Shared per-entry message-id counters for default-mode (aggregated) wrap
/// channels: all instances at one call site draw ids from one sequence, so
/// `msg_id` stays unique within the entry and send/receive log pairing (and
/// the msg-0 rate anchor) stay exact. Bounded by call-site count; `iter =
/// true` instances keep a local counter instead (their entries are already
/// per-instance).
static CHANNEL_MSG_COUNTERS: OnceLock<StdRwLock<HashMap<u32, Arc<AtomicU64>>>> = OnceLock::new();

pub(crate) fn entry_msg_counter(id: u32) -> Arc<AtomicU64> {
    let map = CHANNEL_MSG_COUNTERS.get_or_init(|| StdRwLock::new(HashMap::new()));
    if let Some(counter) = map.read().unwrap().get(&id) {
        return Arc::clone(counter);
    }
    let mut writer = map.write().unwrap();
    Arc::clone(
        writer
            .entry(id)
            .or_insert_with(|| Arc::new(AtomicU64::new(0))),
    )
}

/// Send-side sampling decision; `None` skips the clock read and travels in
/// the payload so the receiver skips its read too.
#[inline]
pub(crate) fn sample_stamp() -> Option<Instant> {
    crate::lib_on::sampling::channels_should_time().then(Instant::now)
}

/// Event timestamp for a wrap send. Unsampled events are stamped at worker
/// drain time, except msg 0: its send always gets a real stamp so
/// `first_msg_ns` anchors throughput rates exactly at every sampling rate,
/// including count-only mode. The payload stamp (and thus the delay) is
/// unaffected.
#[inline]
pub(crate) fn anchor_first_msg(msg_id: u64, sent_at: Option<Instant>) -> Option<Instant> {
    sent_at.or_else(|| (msg_id == 0).then(Instant::now))
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn timestamp_nanos(timestamp: Instant) -> u64 {
    let start_time = START_TIME.get().copied().unwrap_or(timestamp);
    timestamp.duration_since(start_time).as_nanos() as u64
}

/// Statistics for a single instrumented channel.
#[derive(Debug)]
pub(crate) struct ChannelEntry {
    pub(crate) id: u32,
    /// Column-including call-site key (`file:line:column`); the identity used
    /// by the display-suffix scan so same-line call sites do not cross-suffix.
    pub(crate) key: &'static str,
    /// The `file:line` form shown to users.
    pub(crate) source: &'static str,
    pub(crate) label: Option<String>,
    pub(crate) channel_type: ChannelType,
    /// Terminal override set by [`ChannelEvent::Notified`]; the displayed
    /// state is otherwise derived from the instance counters, see [`Self::state`].
    notified: bool,
    /// Number of channel instances aggregated into this entry.
    pub(crate) instances: u64,
    /// Number of aggregated instances whose `Closed` event was processed.
    pub(crate) closed_instances: u64,
    /// Messages still queued when their instance fully tore down; they can
    /// never be received, so the counts-derived queue depth subtracts them.
    abandoned_count: u64,
    pub(crate) sent_count: u64,
    pub(crate) received_count: u64,
    /// Earliest message timestamp (ns since start), shared across both directions.
    /// Anchors the elapsed window used to derive throughput rates.
    first_msg_ns: Option<u64>,
    pub(crate) type_name: &'static str,
    pub(crate) type_size: usize,
    /// Exact channel depth, `None` until the first message event. Derived from
    /// `sent_count - received_count` (converged value order-independent).
    pub(crate) queue_size: Option<usize>,
    pub(crate) max_queue_size: Option<usize>,
    /// Avg denominator is `delay_sampled_count` (one delay recorded per sampled receive).
    pub(crate) delay_total_nanos: u64,
    pub(crate) delay_sampled_count: u64,
    delay_hist: Histogram<u64>,
    pub(crate) iter: u32,
}

#[derive(Debug)]
pub(crate) struct ChannelEntryLogs {
    pub(crate) sent_logs: VecDeque<DataFlowLogEntry>,
    pub(crate) received_logs: VecDeque<DataFlowLogEntry>,
}

impl ChannelEntryLogs {
    fn new() -> Self {
        Self {
            sent_logs: VecDeque::with_capacity(*LOGS_LIMIT),
            received_logs: VecDeque::with_capacity(*LOGS_LIMIT),
        }
    }
}

pub(crate) struct ChannelsInternalState {
    pub(crate) stats: HashMap<u32, ChannelEntry>,
    pub(crate) logs: HashMap<u32, ChannelEntryLogs>,
}

pub(crate) fn channel_to_json(
    stats: &ChannelEntry,
    percentiles: &[f64],
    now_ns: u64,
    cloud: bool,
) -> JsonChannelEntry {
    let precision = crate::output::Precision::for_cloud(cloud);
    let label = resolve_label(stats.source, stats.label.as_deref(), Some(stats.iter));

    let mut delay_percentiles = HashMap::new();
    let count_only = stats.delay_sampled_count == 0 && stats.received_count > 0;
    for &p in percentiles {
        let value = if count_only {
            "-".to_string()
        } else {
            precision.duration(stats.delay_percentile_nanos(p))
        };
        delay_percentiles.insert(crate::output::format_percentile_key(p), value);
    }
    let delay_avg = if count_only {
        "-".to_string()
    } else {
        precision.duration(stats.delay_avg_nanos())
    };

    JsonChannelEntry {
        id: stats.id,
        source: stats.source.to_string(),
        label,
        has_custom_label: stats.label.is_some(),
        channel_type: stats.channel_type.to_string(),
        state: stats.display_state().map(|s| s.as_str().to_string()),
        instances: stats.instances,
        closed_instances: stats.closed_instances,
        sent_count: stats.sent_count,
        received_count: stats.received_count,
        sent_per_sec: stats.sent_per_sec(now_ns),
        received_per_sec: stats.received_per_sec(now_ns),
        type_name: stats.type_name.to_string(),
        type_size: stats.type_size,
        queue_size: stats.queue_size,
        max_queue_size: stats.max_queue_size,
        delay_avg: Some(delay_avg),
        delay_percentiles,
        delay_sampled_count: Some(stats.delay_sampled_count),
        location: crate::lib_on::locations::location_for_key(stats.key),
        iter: stats.iter,
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl ChannelEntry {
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: u32,
        key: &'static str,
        source: &'static str,
        label: Option<String>,
        channel_type: ChannelType,
        type_name: &'static str,
        type_size: usize,
        iter: u32,
    ) -> Self {
        Self {
            id,
            key,
            source,
            label,
            channel_type,
            notified: false,
            instances: 0,
            closed_instances: 0,
            abandoned_count: 0,
            sent_count: 0,
            received_count: 0,
            first_msg_ns: None,
            type_name,
            type_size,
            queue_size: None,
            max_queue_size: None,
            delay_total_nanos: 0,
            delay_sampled_count: 0,
            delay_hist: Self::new_histogram(),
            iter,
        }
    }

    const LOW_NS: u64 = 1;
    const HIGH_NS: u64 = crate::lib_on::MAX_DURATION_NS;
    const SIGFIGS: u8 = 3;

    fn new_histogram() -> Histogram<u64> {
        Histogram::<u64>::new_with_bounds(Self::LOW_NS, Self::HIGH_NS, Self::SIGFIGS)
            .expect("hdrhistogram init")
    }

    #[inline]
    fn record_delay(&mut self, nanos: u64) {
        self.delay_sampled_count += 1;
        self.delay_total_nanos += nanos;
        self.delay_hist
            .record(nanos.clamp(Self::LOW_NS, Self::HIGH_NS))
            .unwrap();
    }

    /// Bucket projections of the sampled processing delays for the Prometheus
    /// exporter (empty without samples).
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn native_delay_buckets(&self, schema: i32) -> Vec<(i32, u64)> {
        crate::lib_on::native_histograms::native_buckets_opt(
            Some(&self.delay_hist),
            self.delay_sampled_count > 0,
            schema,
            crate::lib_on::native_histograms::NANOS_SCALE,
        )
    }

    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn classic_delay_buckets(&self, boundaries: &[u64]) -> Vec<u64> {
        crate::lib_on::native_histograms::classic_buckets_opt(
            Some(&self.delay_hist),
            self.delay_sampled_count > 0,
            boundaries,
        )
    }

    #[inline]
    fn record_activity(&mut self, ts_ns: u64) {
        // Per-thread batch flushing can deliver events out of timestamp order, so
        // track the minimum rather than the first processed.
        self.first_msg_ns = Some(self.first_msg_ns.map_or(ts_ns, |first| first.min(ts_ns)));
    }

    fn rate_per_sec(&self, count: u64, now_ns: u64) -> Option<f64> {
        // A oneshot carries a single message, so any rate is meaningless.
        if self.channel_type == ChannelType::Oneshot {
            return None;
        }
        // Below two messages the distribution is too sparse to report a rate.
        if count < 2 {
            return None;
        }
        // Divide by elapsed time since the first message, not the first-to-last
        // span: the span collapses to one gap for sparse channels.
        let first = self.first_msg_ns?;
        let elapsed_ns = now_ns.checked_sub(first)?;
        if elapsed_ns == 0 {
            return None;
        }
        Some(count as f64 / (elapsed_ns as f64 / 1e9))
    }

    /// `now_ns` is the report's elapsed timestamp so the rate window never
    /// exceeds the `current_elapsed_ns` published alongside the entry.
    pub(crate) fn sent_per_sec(&self, now_ns: u64) -> Option<f64> {
        self.rate_per_sec(self.sent_count, now_ns)
    }

    pub(crate) fn received_per_sec(&self, now_ns: u64) -> Option<f64> {
        self.rate_per_sec(self.received_count, now_ns)
    }

    pub(crate) fn delay_avg_nanos(&self) -> u64 {
        self.delay_total_nanos
            .checked_div(self.delay_sampled_count)
            .unwrap_or(0)
    }

    pub(crate) fn delay_percentile_nanos(&self, p: f64) -> u64 {
        if self.delay_sampled_count == 0 {
            return 0;
        }
        self.delay_hist.value_at_percentile(p.clamp(0.0, 100.0))
    }

    /// Counts-derived in-flight depth, net of messages abandoned in fully
    /// torn-down instances. Transiently saturates to zero when an `Abandoned`
    /// batch is drained ahead of its instance's send events; converges once
    /// the queues are swept.
    fn outstanding_depth(&self) -> usize {
        self.sent_count
            .saturating_sub(self.received_count)
            .saturating_sub(self.abandoned_count) as usize
    }

    /// Current depth is counts-derived (`sent - received`), exact once the channel
    /// is idle since the counters commute, but it can transiently overshoot when a
    /// producer batch reaches the worker ahead of the matching consumer batch.
    ///
    /// For a single instance the peak comes only from real `len()` snapshots; max
    /// of those is order-independent, so it stays a true high-water mark, and
    /// current is clamped to it so `current <= max`. For an aggregated entry
    /// (`instances > 1`) a single-instance `len()` undercounts when several
    /// instances hold messages at once, so the peak also tracks the counts-derived
    /// combined depth and means "peak combined depth" instead.
    fn record_queue(&mut self, queue_len: usize) {
        let mut max = self.max_queue_size.unwrap_or(0).max(queue_len);
        let depth = self.outstanding_depth();
        if self.instances > 1 {
            max = max.max(depth);
        }
        self.max_queue_size = Some(max);
        self.queue_size = Some(depth.min(max));
    }

    /// Re-derives the displayed depth after a non-message event changes what
    /// the counts mean (a late `Created`/`Instance` backfill lifting the
    /// single-instance clamp, or `Abandoned` retiring messages). Without this,
    /// a call site whose channels go idle before their registration events are
    /// swept would keep a stale clamped depth until the next send/receive.
    fn refresh_queue(&mut self) {
        if self.queue_size.is_none() {
            return;
        }
        let depth = self.outstanding_depth();
        let mut max = self.max_queue_size.unwrap_or(0);
        if self.instances > 1 {
            max = max.max(depth);
        }
        self.max_queue_size = Some(max);
        self.queue_size = Some(depth.min(max));
    }

    /// State shown to consumers: `None` for aggregated entries (`instances >
    /// 1`), whose instances open and close independently - a single
    /// Active/Closed flag would flicker with churn and mislead. Single-instance
    /// and `iter = true` entries keep their exact state.
    pub(crate) fn display_state(&self) -> Option<ChannelState> {
        (self.instances <= 1).then(|| self.state())
    }

    /// Displayed state, derived from the instance counters. `>=` rather than
    /// `==` because a `Closed` event can be drained before the `Instance` event
    /// from another thread; the counters converge once idle. A processed
    /// `Notified` stays terminal unless every instance has closed.
    pub(crate) fn state(&self) -> ChannelState {
        if self.instances > 0 && self.closed_instances >= self.instances {
            ChannelState::Closed
        } else if self.notified {
            ChannelState::Notified
        } else {
            ChannelState::Active
        }
    }
}

/// Events sent to the background channel statistics collection thread.
#[derive(Debug)]
pub(crate) enum ChannelEvent {
    Created {
        id: u32,
        /// Column-including call-site key (`file:line:column`); distinguishes
        /// same-line invocations in the display-suffix scan. `source` is the
        /// `file:line` shown to users.
        key: &'static str,
        source: &'static str,
        display_label: Option<String>,
        channel_type: ChannelType,
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
    /// `timestamp` is `None` for messages unsampled under time sampling;
    /// the worker stamps those at drain time.
    WrapMessageSent {
        id: u32,
        msg_id: u64,
        log: Option<String>,
        timestamp: Option<Instant>,
        queue_len: usize,
    },
    WrapMessageReceived {
        id: u32,
        msg_id: u64,
        timestamp: Option<Instant>,
        queue_len: usize,
        delay_nanos: Option<u64>,
    },
    Closed {
        id: u32,
    },
    /// Messages left queued in an instance when its second endpoint dropped;
    /// they can never be received.
    Abandoned {
        id: u32,
        count: u64,
    },
    #[allow(dead_code)]
    Notified {
        id: u32,
    },
}

pub(crate) struct ChannelsState {
    pub(crate) inner: Arc<MetaRwLock<ChannelsInternalState>>,
    pub(crate) shutdown_tx: Mutex<Option<CbSender<()>>>,
    pub(crate) completion_rx: Mutex<Option<CbReceiver<()>>>,
}

pub(crate) static CHANNELS_STATE: OnceLock<ChannelsState> = OnceLock::new();

pub(crate) use crate::lib_on::START_TIME;

pub(crate) use crate::lib_on::hotpath_guard::LOGS_LIMIT;

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
/// Entry for events that arrive ahead of their `Created` (sweeps only preserve
/// per-thread order, so another thread's data events can be drained first).
/// `Created` backfills the metadata.
fn placeholder_channel_entry(id: u32) -> ChannelEntry {
    ChannelEntry::new(id, "", "", None, ChannelType::Pending, "", 0, 0)
}

fn process_channel_event(state: &mut ChannelsInternalState, event: ChannelEvent) {
    match event {
        ChannelEvent::Created {
            id,
            key,
            source,
            display_label,
            channel_type,
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
                .or_insert_with(|| placeholder_channel_entry(id));
            entry.key = key;
            entry.source = source;
            entry.label = display_label;
            entry.channel_type = channel_type;
            entry.type_name = type_name;
            entry.type_size = type_size;
            entry.iter = iter;
            entry.instances += 1;
            entry.refresh_queue();
            state.logs.entry(id).or_insert_with(ChannelEntryLogs::new);
        }
        ChannelEvent::Instance { id } => {
            let entry = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_channel_entry(id));
            entry.instances += 1;
            entry.refresh_queue();
        }
        ChannelEvent::WrapMessageSent {
            id,
            msg_id,
            log,
            timestamp,
            queue_len,
        } => {
            // Unsampled events are stamped at drain time; `first_msg_ns` stays
            // exact because msg 0's send always carries a real stamp
            // (`anchor_first_msg`), at every rate including count-only.
            let ts_ns = timestamp
                .map(timestamp_nanos)
                .unwrap_or_else(crate::lib_on::current_elapsed_ns);
            let channel_stats = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_channel_entry(id));
            channel_stats.sent_count += 1;
            channel_stats.record_activity(ts_ns);
            channel_stats.record_queue(queue_len);
            let sent_count = channel_stats.sent_count;

            let entry_logs = state.logs.entry(id).or_insert_with(ChannelEntryLogs::new);
            let limit = *LOGS_LIMIT;
            if entry_logs.sent_logs.len() >= limit {
                entry_logs.sent_logs.pop_front();
            }
            entry_logs.sent_logs.push_back(DataFlowLogEntry::new(
                sent_count,
                ts_ns,
                log,
                None,
                Some(msg_id),
                None,
            ));
        }
        ChannelEvent::WrapMessageReceived {
            id,
            msg_id,
            timestamp,
            queue_len,
            delay_nanos,
        } => {
            let ts_ns = timestamp
                .map(timestamp_nanos)
                .unwrap_or_else(crate::lib_on::current_elapsed_ns);
            let channel_stats = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_channel_entry(id));
            channel_stats.received_count += 1;
            channel_stats.record_activity(ts_ns);
            channel_stats.record_queue(queue_len);
            if let Some(delay_nanos) = delay_nanos {
                channel_stats.record_delay(delay_nanos);
            }
            let received_count = channel_stats.received_count;

            let entry_logs = state.logs.entry(id).or_insert_with(ChannelEntryLogs::new);
            let limit = *LOGS_LIMIT;
            if entry_logs.received_logs.len() >= limit {
                entry_logs.received_logs.pop_front();
            }
            entry_logs.received_logs.push_back(DataFlowLogEntry::new(
                received_count,
                ts_ns,
                None,
                None,
                Some(msg_id),
                delay_nanos,
            ));
        }
        ChannelEvent::Closed { id } => {
            state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_channel_entry(id))
                .closed_instances += 1;
        }
        ChannelEvent::Abandoned { id, count } => {
            let channel_stats = state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_channel_entry(id));
            channel_stats.abandoned_count += count;
            // No further events arrive for the dead instance, so refresh the
            // displayed depth here instead of waiting for another send/receive.
            channel_stats.refresh_queue();
        }
        ChannelEvent::Notified { id } => {
            state
                .stats
                .entry(id)
                .or_insert_with(|| placeholder_channel_entry(id))
                .notified = true;
        }
    }
}

fn flush_channel_buffer(
    buffer: &mut Vec<ChannelEvent>,
    inner: &Arc<MetaRwLock<ChannelsInternalState>>,
) {
    if buffer.is_empty() {
        return;
    }
    if let Ok(mut shared) = inner.write() {
        for e in buffer.drain(..) {
            process_channel_event(&mut shared, e);
        }
    }
}

/// Initialize the channel statistics collection system (called on first instrumented channel).
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
pub(crate) fn init_channels_state() -> &'static ChannelsState {
    CHANNELS_STATE.get_or_init(|| {
        START_TIME.get_or_init(Instant::now);

        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let (completion_tx, completion_rx) = bounded::<()>(1);
        let inner = Arc::new(meta_rw_lock!(
            "channels_state",
            ChannelsInternalState {
                stats: HashMap::new(),
                logs: HashMap::new(),
            },
        ));
        let inner_clone = Arc::clone(&inner);

        EVENT_QUEUES.set_active(true);

        std::thread::Builder::new()
            .name("hp-channels".into())
            .spawn(move || {
                let flush_interval = std::time::Duration::from_millis(*DRAIN_INTERVAL_MS);
                let mut swept: Vec<ChannelEvent> = Vec::new();

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
                        flush_channel_buffer(&mut swept, &inner_clone);
                        break;
                    }

                    EVENT_QUEUES.sweep(&mut swept);
                    flush_channel_buffer(&mut swept, &inner_clone);
                }

                let _ = completion_tx.send(());
            })
            .expect("Failed to spawn channel-stats-collector thread");

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        ChannelsState {
            inner,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            completion_rx: Mutex::new(Some(completion_rx)),
        }
    })
}

/// Wrapper macros identify a call site as `file:line:column` so two
/// invocations on one physical line cannot alias; this strips the column back
/// to the `file:line` form shown to users.
pub(crate) fn display_source(key: &'static str) -> &'static str {
    key.rsplit_once(':').map_or(key, |(source, _)| source)
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn resolve_label(id: &'static str, provided: Option<&str>, iter: Option<u32>) -> String {
    let base_label = if let Some(l) = provided {
        l.to_string()
    } else if let Some(pos) = id.rfind(':') {
        let (path, line_part) = id.split_at(pos);
        let line = &line_part[1..];
        format!("{}:{}", extract_filename(path), line)
    } else {
        extract_filename(id)
    };

    match iter {
        Some(i) if i > 0 => format!("{}-{}", base_label, i + 1),
        _ => base_label,
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn extract_filename(path: &str) -> String {
    let mut parts = path.rsplitn(3, '/');
    match (parts.next(), parts.next()) {
        (Some(last), Some(second_last)) => format!("{}/{}", second_last, last),
        _ => path.to_string(),
    }
}

/// Trait for instrumenting channels by wrapping their endpoints directly.
///
/// Returns wrapper types (`hotpath::wrap::<backend>::{Sender, Receiver}`) instead of
/// the original channel types, so queue depth is measured exactly. Not intended for
/// direct use; the `channel!` macro dispatches here.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "channel type `{Self}` cannot be instrumented by `channel!`",
    note = "supported: `(Sender, Receiver)` tuples from std, crossbeam, flume, async-channel, tokio (mpsc, oneshot) and futures_channel (mpsc, oneshot)"
)]
pub trait InstrumentChannelWrap {
    type Output;
    fn instrument_wrap(
        self,
        source: &'static str,
        label: Option<String>,
        capacity: Option<usize>,
        iter: bool,
    ) -> Self::Output;
}

impl<C: WrapChannel> InstrumentChannelWrap for C {
    type Output = C::Output;
    fn instrument_wrap(
        self,
        source: &'static str,
        label: Option<String>,
        capacity: Option<usize>,
        iter: bool,
    ) -> Self::Output {
        self.wrap(source, label, capacity, None, iter)
    }
}

/// Trait for instrumenting channels by wrapping their endpoints, with message logging.
///
/// This trait is not intended for direct use. Use the `channel!` macro with
/// `log = true`.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "channel type `{Self}` cannot be instrumented by `channel!`",
    note = "supported: `(Sender, Receiver)` tuples from std, crossbeam, flume, async-channel, tokio (mpsc, oneshot) and futures_channel (mpsc, oneshot); `log = true` needs `T: Debug`"
)]
pub trait InstrumentChannelWrapLog {
    type Output;
    fn instrument_wrap_log(
        self,
        source: &'static str,
        label: Option<String>,
        capacity: Option<usize>,
        iter: bool,
    ) -> Self::Output;
}

impl<C: WrapChannel> InstrumentChannelWrapLog for C
where
    C::Msg: std::fmt::Debug,
{
    type Output = C::Output;
    fn instrument_wrap_log(
        self,
        source: &'static str,
        label: Option<String>,
        capacity: Option<usize>,
        iter: bool,
    ) -> Self::Output {
        let log_fn: fn(&C::Msg) -> String = |m| crate::output_on::format_debug_truncated(m);
        self.wrap(source, label, capacity, Some(log_fn), iter)
    }
}

/// Instrument a channel creation for profiling.
///
/// The macro **wraps the endpoints**: it returns
/// `hotpath::wrap::<backend>::{Sender, Receiver}` instead of the raw channel types and
/// measures exact queue depth plus send->receive latency inline.
///
/// Optional parameters: `label`, `log = true`, `capacity`, `iter = true` (in any
/// order). `log = true` requires `Debug` on the message type.
///
/// # Call-site aggregation and `iter = true`
///
/// By default all channels created at one `channel!` call site (with the same
/// message type) accumulate into a **single entry**: counts and the latency
/// histogram are summed across instances and the `Inst` column reports how
/// many instances the entry aggregates, so profiler state stays bounded even
/// when a call site creates a channel per request. The first registration's
/// `label` and channel kind/capacity win. Aggregated entries show `-` for
/// state (their instances open and close independently); the `Inst` count and
/// `closed_instances` in JSON carry the lifecycle information instead.
///
/// The reported rates are aggregate call-site throughput: total messages
/// divided by elapsed time since the call site's first message. This is a
/// lifetime average - bursty call sites read lower than in-burst throughput.
/// For aggregated entries `Queue` is the combined in-flight depth across live
/// instances (messages abandoned in fully closed instances are subtracted)
/// and `Max queue` the peak combined depth; with a single instance both keep
/// their exact per-channel meaning. Log windows (`log = true`) interleave
/// messages from all instances; message ids are drawn from one per-call-site
/// sequence, so send/receive log pairing stays exact.
///
/// Pass `iter = true` to give every instance its own entry instead (displayed
/// as `label`, `label-2`, `label-3`, ...), e.g. one row per spawned worker
/// with its individual counts and rate. Profiler state then grows with the
/// number of channels ever created, so prefer the default aggregation for
/// call sites with unbounded instance churn.
///
/// # Inline construction
///
/// The channel expression **must be constructed inline**, e.g.
/// `channel!(crossbeam_channel::unbounded::<T>())`. The wrapper rebuilds the inner channel
/// (to carry a per-message id) and discards the one you pass in, so any endpoint cloned
/// before wrapping is orphaned and its messages are silently dropped. Clone the returned
/// wrapper endpoints instead.
///
/// Bounded `std::sync::mpsc` (`sync_channel`) and bounded `futures_channel::mpsc` cannot
/// recover their capacity from the endpoint, so `capacity = N` is required, e.g.
/// `channel!(std::sync::mpsc::sync_channel::<T>(100), capacity = 100)`. **The value must
/// match the constructor argument** - the wrapper rebuilds the inner channel from
/// `capacity`, so a mismatch silently changes backpressure (and only in profiled builds,
/// since with `hotpath` off `channel!` returns your original channel untouched). Neither
/// exposes a capacity accessor, so keep the two numbers equal. Unbounded std, crossbeam,
/// flume, tokio and async-channel wrappers recover the bound from the endpoint and need no
/// `capacity`.
///
/// # Examples
///
/// ```rust,no_run
/// use tokio::sync::mpsc;
///
/// #[tokio::main]
/// async fn main() {
///    let (tx, rx) = hotpath::channel!(mpsc::channel::<String>(100));
///
///    tx.send("Hello".to_string()).await.unwrap();
/// }
/// ```
#[macro_export]
macro_rules! channel {
    ($expr:expr) => {{
        const CHANNEL_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(CHANNEL_ID);
        $crate::InstrumentChannelWrap::instrument_wrap($expr, CHANNEL_ID, None, None, false)
    }};

    // Any argument list is parsed order-independently by the muncher below. Slots are
    // `label capacity log iter`; `label`/`capacity`/`iter` are stored as
    // ready-to-use expression tokens so the dispatch only branches on `log`.
    // `CHANNEL_ID` is captured once here so `file!()`/`line!()` resolve to the user's
    // call site.
    ($expr:expr, $($rest:tt)*) => {{
        const CHANNEL_ID: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(CHANNEL_ID);
        $crate::channel!(@munch CHANNEL_ID, $expr ; (None) (None) [nolog] (false) ; $($rest)*)
    }};

    (@munch $id:ident, $e:expr ; $lbl:tt $cap:tt $log:tt $it:tt ;) => {
        $crate::channel!(@dispatch $id, $e ; $lbl $cap $log $it)
    };
    // A literal label is checked for uniqueness, then re-munched wrapped in a
    // block so it takes the plain expression arm below.
    (@munch $id:ident, $e:expr ; $lbl:tt $cap:tt $log:tt $it:tt ; label = $l:literal $(, $($r:tt)*)?) => {{
        $crate::__unique_label!(channel, $l);
        $crate::channel!(@munch $id, $e ; $lbl $cap $log $it ; label = { $l } $(, $($r)*)?)
    }};
    (@munch $id:ident, $e:expr ; $lbl:tt $cap:tt $log:tt $it:tt ; label = $l:expr $(, $($r:tt)*)?) => {
        $crate::channel!(@munch $id, $e ; (Some($l.to_string())) $cap $log $it ; $($($r)*)?)
    };
    (@munch $id:ident, $e:expr ; $lbl:tt $cap:tt $log:tt $it:tt ; capacity = $c:expr $(, $($r:tt)*)?) => {
        $crate::channel!(@munch $id, $e ; $lbl (Some($c)) $log $it ; $($($r)*)?)
    };
    (@munch $id:ident, $e:expr ; $lbl:tt $cap:tt $log:tt $it:tt ; log = true $(, $($r:tt)*)?) => {
        $crate::channel!(@munch $id, $e ; $lbl $cap [log] $it ; $($($r)*)?)
    };
    (@munch $id:ident, $e:expr ; $lbl:tt $cap:tt $log:tt $it:tt ; iter = true $(, $($r:tt)*)?) => {
        $crate::channel!(@munch $id, $e ; $lbl $cap $log (true) ; $($($r)*)?)
    };

    (@dispatch $id:ident, $e:expr ; $lbl:tt $cap:tt [nolog] $it:tt) => {
        $crate::InstrumentChannelWrap::instrument_wrap($e, $id, $lbl, $cap, $it)
    };
    (@dispatch $id:ident, $e:expr ; $lbl:tt $cap:tt [log] $it:tt) => {
        $crate::InstrumentChannelWrapLog::instrument_wrap_log($e, $id, $lbl, $cap, $it)
    };
}

/// Custom labels come first (sorted alphabetically), then auto-generated labels (sorted by source and iter).
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn compare_channel_entries(a: &ChannelEntry, b: &ChannelEntry) -> std::cmp::Ordering {
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

/// Runs `f` on the entries sorted for display, borrowed under the read lock:
/// nothing is cloned, so the per-entry histograms stay in the map.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
pub(crate) fn with_sorted_channel_entries<R>(f: impl FnOnce(&[&ChannelEntry]) -> R) -> R {
    let Some(state) = CHANNELS_STATE.get() else {
        return f(&[]);
    };
    let guard = state.inner.read().unwrap();
    let mut stats: Vec<&ChannelEntry> = guard.stats.values().collect();
    stats.sort_by(|a, b| compare_channel_entries(a, b));
    f(&stats)
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_channels_json() -> crate::json::JsonChannelsList {
    let percentiles = crate::lib_on::hotpath_guard::configured_percentiles();
    let current_elapsed_ns = crate::lib_on::current_elapsed_ns();
    let data: Vec<crate::json::JsonChannelEntry> = with_sorted_channel_entries(|entries| {
        entries
            .iter()
            .map(|entry| channel_to_json(entry, &percentiles, current_elapsed_ns, false))
            .collect()
    });

    crate::json::JsonChannelsList {
        current_elapsed_ns,
        percentiles,
        total_count: data.len(),
        included_count: data.len(),
        data,
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_channel_logs(id: u32) -> Option<ChannelLogs> {
    let state = CHANNELS_STATE.get()?;
    let guard = state.inner.read().unwrap();
    let entry_logs = guard.logs.get(&id)?;
    Some(ChannelLogs {
        id,
        sent_logs: entry_logs.sent_logs.iter().rev().cloned().collect(),
        received_logs: entry_logs.received_logs.iter().rev().cloned().collect(),
    })
}

#[cfg(test)]
mod tests {
    use crate::channels::{
        process_channel_event, ChannelEvent, ChannelState, ChannelType, ChannelsInternalState,
    };
    use crate::instant::Instant;
    use std::collections::HashMap;

    /// Current depth is counts-derived, so it converges exactly regardless of arrival
    /// order even when per-thread batches reach the worker out of order with equal
    /// `Instant` timestamps. Peak tracks the real `len()` snapshot and current is
    /// clamped to it, so `current <= max`.
    #[test]
    fn out_of_order_queue_snapshot_converges_within_peak() {
        let mut state = ChannelsInternalState {
            stats: HashMap::new(),
            logs: HashMap::new(),
        };

        let id = 1;
        process_channel_event(
            &mut state,
            ChannelEvent::Created {
                id,
                key: "test",
                source: "test",
                display_label: None,
                channel_type: ChannelType::Unbounded,
                type_name: "u8",
                type_size: 1,
                iter_mode: false,
            },
        );

        // Same tick for both ops; the receive batch arrives first, then the send batch.
        let ts = Instant::now();
        process_channel_event(
            &mut state,
            ChannelEvent::WrapMessageReceived {
                id,
                msg_id: 1,
                timestamp: Some(ts),
                queue_len: 0,
                delay_nanos: Some(0),
            },
        );
        process_channel_event(
            &mut state,
            ChannelEvent::WrapMessageSent {
                id,
                msg_id: 1,
                log: None,
                timestamp: Some(ts),
                queue_len: 1,
            },
        );

        let entry = state.stats.get(&id).expect("channel registered");

        assert_eq!(
            entry.queue_size,
            Some(0),
            "current depth must equal sent_count - received_count regardless of order"
        );

        // Peak is the real `len()` snapshot (1); current is clamped to it.
        assert_eq!(entry.max_queue_size, Some(1));
        assert!(entry.queue_size <= entry.max_queue_size);
    }

    /// The displayed state is derived from `instances` vs `closed_instances`,
    /// so a `Closed` drained ahead of the matching `Instance` (cross-thread
    /// sweep order) still converges: `closed >= instances` marks the entry
    /// Closed early, and the late `Instance` of a still-open channel flips it
    /// back to Active until that instance closes too.
    #[test]
    fn out_of_order_instance_close_state_converges() {
        let mut state = ChannelsInternalState {
            stats: HashMap::new(),
            logs: HashMap::new(),
        };

        let id = 1;
        process_channel_event(
            &mut state,
            ChannelEvent::Created {
                id,
                key: "test",
                source: "test",
                display_label: None,
                channel_type: ChannelType::Unbounded,
                type_name: "u8",
                type_size: 1,
                iter_mode: false,
            },
        );
        assert_eq!(state.stats[&id].state(), ChannelState::Active);

        // Second instance's Closed arrives before its Instance event.
        process_channel_event(&mut state, ChannelEvent::Closed { id });
        process_channel_event(&mut state, ChannelEvent::Closed { id });
        assert_eq!(state.stats[&id].state(), ChannelState::Closed);

        // The late Instance belongs to a third, still-open channel: not fully
        // closed anymore.
        process_channel_event(&mut state, ChannelEvent::Instance { id });
        process_channel_event(&mut state, ChannelEvent::Instance { id });
        assert_eq!(state.stats[&id].instances, 3);
        assert_eq!(state.stats[&id].state(), ChannelState::Active);

        process_channel_event(&mut state, ChannelEvent::Closed { id });
        assert_eq!(state.stats[&id].state(), ChannelState::Closed);
    }

    /// A `Closed` processed on a placeholder entry (before any `Created`)
    /// must not display as Closed while `instances == 0`.
    #[test]
    fn closed_before_created_stays_active_until_backfill() {
        let mut state = ChannelsInternalState {
            stats: HashMap::new(),
            logs: HashMap::new(),
        };

        let id = 7;
        process_channel_event(&mut state, ChannelEvent::Closed { id });
        assert_eq!(state.stats[&id].state(), ChannelState::Active);

        process_channel_event(
            &mut state,
            ChannelEvent::Created {
                id,
                key: "test",
                source: "test",
                display_label: None,
                channel_type: ChannelType::Oneshot,
                type_name: "u8",
                type_size: 1,
                iter_mode: false,
            },
        );
        assert_eq!(state.stats[&id].state(), ChannelState::Closed);
    }

    /// Sends swept ahead of their registration events land on a placeholder
    /// with at most one known instance, so the depth is clamped to a single
    /// channel's `len()`. The late `Created`/`Instance` backfill must lift the
    /// clamp even if the channels then stay idle (no further queue events).
    #[test]
    fn late_instance_event_refreshes_combined_queue_depth() {
        let mut state = ChannelsInternalState {
            stats: HashMap::new(),
            logs: HashMap::new(),
        };

        let id = 1;
        let ts = Instant::now();
        for msg_id in 0..2 {
            process_channel_event(
                &mut state,
                ChannelEvent::WrapMessageSent {
                    id,
                    msg_id,
                    log: None,
                    timestamp: Some(ts),
                    queue_len: 1,
                },
            );
        }
        // Single-instance clamp while the registrations are still unswept.
        assert_eq!(state.stats[&id].queue_size, Some(1));

        process_channel_event(
            &mut state,
            ChannelEvent::Created {
                id,
                key: "test",
                source: "test",
                display_label: None,
                channel_type: ChannelType::Unbounded,
                type_name: "u8",
                type_size: 1,
                iter_mode: false,
            },
        );
        process_channel_event(&mut state, ChannelEvent::Instance { id });

        let entry = state.stats.get(&id).expect("channel registered");
        assert_eq!(entry.queue_size, Some(2), "combined depth after backfill");
        assert_eq!(entry.max_queue_size, Some(2));
    }

    /// Messages abandoned in a fully torn-down instance are reconciled via
    /// `Abandoned`, so sequential instances that each abandon a message do not
    /// accumulate a phantom queue depth at the call site.
    #[test]
    fn abandoned_messages_do_not_inflate_queue_depth() {
        let mut state = ChannelsInternalState {
            stats: HashMap::new(),
            logs: HashMap::new(),
        };

        let id = 1;
        process_channel_event(
            &mut state,
            ChannelEvent::Created {
                id,
                key: "test",
                source: "test",
                display_label: None,
                channel_type: ChannelType::Bounded(1),
                type_name: "u8",
                type_size: 1,
                iter_mode: false,
            },
        );

        // Instance 1 sends one message that is never received, then fully
        // tears down: first endpoint drop emits Closed, second Abandoned.
        let ts = Instant::now();
        process_channel_event(
            &mut state,
            ChannelEvent::WrapMessageSent {
                id,
                msg_id: 0,
                log: None,
                timestamp: Some(ts),
                queue_len: 1,
            },
        );
        process_channel_event(&mut state, ChannelEvent::Closed { id });
        process_channel_event(&mut state, ChannelEvent::Abandoned { id, count: 1 });
        assert_eq!(
            state.stats[&id].queue_size,
            Some(0),
            "abandoned message must not linger as in-flight depth"
        );

        // Instance 2 holds one message: depth reflects only its message, and
        // the peak does not inherit the dead instance's deficit.
        process_channel_event(&mut state, ChannelEvent::Instance { id });
        process_channel_event(
            &mut state,
            ChannelEvent::WrapMessageSent {
                id,
                msg_id: 1,
                log: None,
                timestamp: Some(ts),
                queue_len: 1,
            },
        );
        let entry = state.stats.get(&id).expect("channel registered");
        assert_eq!(entry.queue_size, Some(1));
        assert_eq!(entry.max_queue_size, Some(1));
    }

    /// Aggregated entries (`instances > 1`) track the counts-derived combined
    /// depth in the peak, so concurrent instances each holding messages are
    /// not clamped down to the largest single-instance `len()` snapshot.
    #[test]
    fn aggregated_queue_peak_tracks_combined_depth() {
        let mut state = ChannelsInternalState {
            stats: HashMap::new(),
            logs: HashMap::new(),
        };

        let id = 1;
        process_channel_event(
            &mut state,
            ChannelEvent::Created {
                id,
                key: "test",
                source: "test",
                display_label: None,
                channel_type: ChannelType::Unbounded,
                type_name: "u8",
                type_size: 1,
                iter_mode: false,
            },
        );
        process_channel_event(&mut state, ChannelEvent::Instance { id });

        // Two instances each hold one message; each reports its own len() of 1.
        let ts = Instant::now();
        for msg_id in 0..2 {
            process_channel_event(
                &mut state,
                ChannelEvent::WrapMessageSent {
                    id,
                    msg_id,
                    log: None,
                    timestamp: Some(ts),
                    queue_len: 1,
                },
            );
        }

        let entry = state.stats.get(&id).expect("channel registered");
        assert_eq!(entry.queue_size, Some(2), "combined in-flight depth");
        assert_eq!(entry.max_queue_size, Some(2), "peak combined depth");
        assert!(entry.queue_size <= entry.max_queue_size);
    }

    /// The iter display-suffix scan counts by the column-including key, so two
    /// `iter = true` call sites on one physical line (same display source,
    /// different columns) each start their own suffix sequence instead of the
    /// second one rendering as a `-2` instance of the first.
    #[test]
    fn iter_suffix_scan_counts_by_column_key() {
        let mut state = ChannelsInternalState {
            stats: HashMap::new(),
            logs: HashMap::new(),
        };

        let created = |id, key| ChannelEvent::Created {
            id,
            key,
            source: "f.rs:1",
            display_label: None,
            channel_type: ChannelType::Unbounded,
            type_name: "u8",
            type_size: 1,
            iter_mode: true,
        };

        // Two distinct call sites sharing one physical line.
        process_channel_event(&mut state, created(1, "f.rs:1:14"));
        process_channel_event(&mut state, created(2, "f.rs:1:52"));
        assert_eq!(state.stats[&1].iter, 0);
        assert_eq!(
            state.stats[&2].iter, 0,
            "same-line site must not cross-suffix"
        );

        // A repeat instance at the first site still gets the -2 suffix.
        process_channel_event(&mut state, created(3, "f.rs:1:14"));
        assert_eq!(state.stats[&3].iter, 1);
    }
}
