//! Futures instrumentation module - tracks async Future lifecycle and poll statistics.

use crate::batch::{EventProducer, EventQueueRegistry};
use crate::channels::{resolve_label, LOGS_LIMIT, START_TIME};
use crate::metrics_server::METRICS_SERVER_PORT;
use crossbeam_channel::{bounded, Receiver as CbReceiver, RecvTimeoutError, Sender as CbSender};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::instant::Instant;

use crate::lib_on::hotpath_guard::DRAIN_INTERVAL_MS;
use crate::lib_on::{meta_rw_lock, MetaRwLock};

pub(crate) mod wrapper;

pub use wrapper::{InstrumentedFuture, InstrumentedFutureLog};

use crate::json::JsonFutureEntry;
pub(crate) use crate::json::{FutureLog, FutureLogsList, FutureState};
pub use crate::Format;

pub(crate) static FUTURE_CALL_ID_COUNTER: AtomicU32 = AtomicU32::new(1);
static FUTURE_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn next_future_id() -> u32 {
    FUTURE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

use std::sync::LazyLock;

static SOURCE_TO_FUTURE_ID: LazyLock<MetaRwLock<HashMap<&'static str, u32>>> =
    LazyLock::new(|| meta_rw_lock!("futures_source_ids", HashMap::new()));

/// Get or create a future_id for a source location.
/// Returns (future_id, is_new) where is_new indicates if this is a newly created future.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_or_create_future_id(source: &'static str) -> (u32, bool) {
    let map = &*SOURCE_TO_FUTURE_ID;

    {
        let read_guard = map.read().unwrap();
        if let Some(&future_id) = read_guard.get(source) {
            return (future_id, false);
        }
    }

    let mut write_guard = map.write().unwrap();

    if let Some(&future_id) = write_guard.get(source) {
        return (future_id, false);
    }

    let future_id = next_future_id();
    write_guard.insert(source, future_id);
    (future_id, true)
}

/// Aggregated statistics for a source location.
#[derive(Debug, Clone)]
pub(crate) struct FutureEntry {
    pub(crate) id: u32,
    pub(crate) source: &'static str,
    pub(crate) label: Option<String>,
    pub(crate) logs_count: u64,
    pub(crate) total_poll_count: u64,
    pub(crate) sampled_polls: u64,
    pub(crate) total_poll_duration_ns: u64,
    pub(crate) total_poll_alloc_bytes: Option<u64>,
    pub(crate) total_poll_alloc_count: Option<u64>,
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl FutureEntry {
    fn new(id: u32, source: &'static str, label: Option<String>) -> Self {
        Self {
            id,
            source,
            label,
            logs_count: 0,
            total_poll_count: 0,
            sampled_polls: 0,
            total_poll_duration_ns: 0,
            total_poll_alloc_bytes: None,
            total_poll_alloc_count: None,
        }
    }

    pub(crate) fn total_polls(&self) -> u64 {
        self.total_poll_count
    }

    pub(crate) fn avg_poll_duration_ns(&self) -> Option<u64> {
        self.total_poll_duration_ns.checked_div(self.sampled_polls)
    }

    /// Exact when every poll was timed, extrapolated (`avg * polls`) under time sampling.
    pub(crate) fn display_total_poll_duration_ns(&self) -> u64 {
        if self.sampled_polls == self.total_poll_count {
            self.total_poll_duration_ns
        } else {
            self.avg_poll_duration_ns().unwrap_or(0) * self.total_poll_count
        }
    }

    pub(crate) fn total_poll_alloc_bytes(&self) -> Option<u64> {
        self.total_poll_alloc_bytes
    }

    pub(crate) fn total_poll_alloc_count(&self) -> Option<u64> {
        self.total_poll_alloc_count
    }
}

#[derive(Debug)]
pub(crate) struct FutureEntryLogs {
    pub(crate) logs: VecDeque<FutureLog>,
}

impl FutureEntryLogs {
    fn new() -> Self {
        Self {
            logs: VecDeque::with_capacity(*LOGS_LIMIT),
        }
    }

    fn find_call_mut(&mut self, id: u32) -> Option<&mut FutureLog> {
        self.logs.iter_mut().find(|c| c.id == id)
    }

    /// Registers a call log entry if absent, whether the call arrives via
    /// `CallCreated` or via a data event that outran it on another thread's
    /// queue. The window is best-effort: a call evicted by `LOGS_LIMIT` can be
    /// re-registered (with reset per-call details) by a late event; call
    /// counts and poll totals live in `FutureEntry` and are unaffected.
    fn register_call(&mut self, future_id: u32, call_id: u32) {
        if self.logs.iter().any(|c| c.id == call_id) {
            return;
        }
        if self.logs.len() >= *LOGS_LIMIT {
            self.logs.pop_front();
        }
        self.logs.push_back(FutureLog::new(call_id, future_id));
    }
}

pub(crate) struct FuturesInternalState {
    pub(crate) stats: HashMap<u32, FutureEntry>,
    pub(crate) logs: HashMap<u32, FutureEntryLogs>,
}

/// `future!` ids are `file:line:column` so same-line invocations cannot
/// alias; this strips the column back to the `file:line` form shown to users.
/// Name-based ids (`#[future_fn]`, `#[measure(future = true)]`) pass through:
/// their trailing segments are never both numeric.
pub(crate) fn display_source(id: &'static str) -> &'static str {
    let mut parts = id.rsplitn(3, ':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(column), Some(line), Some(_path))
            if !column.is_empty()
                && !line.is_empty()
                && column.bytes().all(|b| b.is_ascii_digit())
                && line.bytes().all(|b| b.is_ascii_digit()) =>
        {
            &id[..id.len() - column.len() - 1]
        }
        _ => id,
    }
}

impl From<&FutureEntry> for JsonFutureEntry {
    fn from(stats: &FutureEntry) -> Self {
        let source = display_source(stats.source);
        let label = resolve_label(source, stats.label.as_deref(), None);

        JsonFutureEntry {
            id: stats.id,
            source: source.to_string(),
            label,
            has_custom_label: stats.label.is_some(),
            call_count: stats.logs_count,
            total_polls: stats.total_polls(),
            sampled_polls: stats.sampled_polls,
            total_poll_duration_ns: stats.display_total_poll_duration_ns(),
            total_poll_alloc_bytes: stats.total_poll_alloc_bytes(),
            total_poll_alloc_count: stats.total_poll_alloc_count(),
            location: crate::lib_on::locations::location_for_key(stats.source),
        }
    }
}

/// Result of polling a future.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PollResult {
    Pending,
    Ready,
}

/// Events emitted during the lifecycle of an instrumented future.
#[derive(Debug)]
pub(crate) enum FutureEvent {
    Created {
        future_id: u32,
        source: &'static str,
        display_label: Option<String>,
    },
    CallCreated {
        future_id: u32,
        call_id: u32,
    },
    Polled {
        future_id: u32,
        call_id: u32,
        result: PollResult,
        /// `None` for polls whose duration was not measured under time
        /// sampling; the poll still counts and alloc fields stay exact.
        poll_duration_ns: Option<u64>,
        poll_alloc_bytes: Option<u64>,
        poll_alloc_count: Option<u64>,
    },
    Completed {
        future_id: u32,
        call_id: u32,
        log_message: Option<String>,
    },
    Cancelled {
        future_id: u32,
        call_id: u32,
    },
}

pub(crate) struct FuturesState {
    pub(crate) inner: Arc<MetaRwLock<FuturesInternalState>>,
    pub(crate) shutdown_tx: Mutex<Option<CbSender<()>>>,
    pub(crate) completion_rx: Mutex<Option<CbReceiver<()>>>,
}

pub(crate) static FUTURES_STATE: OnceLock<FuturesState> = OnceLock::new();

static EVENT_QUEUES: EventQueueRegistry<FutureEvent> = EventQueueRegistry::new();

thread_local! {
    static EVENT_PRODUCER: EventProducer<FutureEvent> = EVENT_QUEUES.register();
}

#[inline]
pub(crate) fn send_future_event(event: FutureEvent) {
    if !EVENT_QUEUES.is_active() {
        return;
    }
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let _ = EVENT_PRODUCER.try_with(|producer| producer.push(event));
}

/// Stops producers ahead of the worker's final sweep at shutdown.
pub(crate) fn stop_future_events() {
    EVENT_QUEUES.set_active(false);
}

/// Initialize the futures event collection system (called on first instrumented future).
#[doc(hidden)]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub fn init_futures_state() {
    let _ = get_futures_state();
}

fn flush_future_buffer(
    buffer: &mut Vec<FutureEvent>,
    inner: &Arc<MetaRwLock<FuturesInternalState>>,
) {
    if buffer.is_empty() {
        return;
    }
    if let Ok(mut shared) = inner.write() {
        for e in buffer.drain(..) {
            process_future_event(&mut shared, e);
        }
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
fn get_futures_state() -> &'static FuturesState {
    FUTURES_STATE.get_or_init(|| {
        START_TIME.get_or_init(Instant::now);

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let (completion_tx, completion_rx) = bounded::<()>(1);
        let inner = Arc::new(meta_rw_lock!(
            "futures_state",
            FuturesInternalState {
                stats: HashMap::new(),
                logs: HashMap::new(),
            },
        ));
        let inner_clone = Arc::clone(&inner);

        EVENT_QUEUES.set_active(true);

        crate::lib_on::spawn_profiler_thread("hp-futures", move || {
            let flush_interval = std::time::Duration::from_millis(*DRAIN_INTERVAL_MS);
            let mut swept: Vec<FutureEvent> = Vec::new();

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
                    flush_future_buffer(&mut swept, &inner_clone);
                    break;
                }

                EVENT_QUEUES.sweep(&mut swept);
                flush_future_buffer(&mut swept, &inner_clone);
            }

            let _ = completion_tx.send(());
        })
        .expect("Failed to spawn futures event collector thread");

        FuturesState {
            inner,
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
            completion_rx: Mutex::new(Some(completion_rx)),
        }
    })
}

/// Ensures the futures worker is running so events have somewhere to flush to.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn ensure_futures_state() {
    let _ = get_futures_state();
}

fn update_future_state(call: &mut FutureLog, state: FutureState) {
    if !matches!(call.state, FutureState::Ready | FutureState::Cancelled) {
        call.state = state;
    }
}

/// Entry for events that arrive ahead of their `Created` (sweeps only preserve
/// per-thread order, so another thread's data events can be drained first).
/// `Created` backfills the metadata.
fn placeholder_future_entry(future_id: u32) -> FutureEntry {
    FutureEntry::new(future_id, "", None)
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn process_future_event(state: &mut FuturesInternalState, event: FutureEvent) {
    fn add_optional(total: &mut Option<u64>, delta: Option<u64>) {
        if let Some(delta) = delta {
            *total = Some(total.unwrap_or(0) + delta);
        }
    }

    // Any event may outrun its `Created`/`CallCreated` (sweeps only preserve
    // per-thread order), so every arm materializes missing entries and the
    // lifecycle events backfill metadata instead of blindly inserting.
    match event {
        FutureEvent::Created {
            future_id,
            source,
            display_label,
        } => {
            let entry = state
                .stats
                .entry(future_id)
                .or_insert_with(|| placeholder_future_entry(future_id));
            entry.source = source;
            entry.label = display_label;
            state
                .logs
                .entry(future_id)
                .or_insert_with(FutureEntryLogs::new);
        }
        FutureEvent::CallCreated { future_id, call_id } => {
            state
                .logs
                .entry(future_id)
                .or_insert_with(FutureEntryLogs::new)
                .register_call(future_id, call_id);
            // `CallCreated` fires exactly once per call, so it alone counts
            // calls - placeholder registration from data events must not.
            state
                .stats
                .entry(future_id)
                .or_insert_with(|| placeholder_future_entry(future_id))
                .logs_count += 1;
        }
        FutureEvent::Polled {
            future_id,
            call_id,
            result,
            poll_duration_ns,
            poll_alloc_bytes,
            poll_alloc_count,
        } => {
            let future_stats = state
                .stats
                .entry(future_id)
                .or_insert_with(|| placeholder_future_entry(future_id));
            future_stats.total_poll_count += 1;
            if let Some(poll_duration_ns) = poll_duration_ns {
                future_stats.sampled_polls += 1;
                future_stats.total_poll_duration_ns += poll_duration_ns;
            }
            add_optional(&mut future_stats.total_poll_alloc_bytes, poll_alloc_bytes);
            add_optional(&mut future_stats.total_poll_alloc_count, poll_alloc_count);

            let entry_logs = state
                .logs
                .entry(future_id)
                .or_insert_with(FutureEntryLogs::new);
            entry_logs.register_call(future_id, call_id);
            if let Some(call) = entry_logs.find_call_mut(call_id) {
                call.poll_count += 1;
                if let Some(poll_duration_ns) = poll_duration_ns {
                    call.sampled_polls += 1;
                    call.total_poll_duration_ns += poll_duration_ns;
                    call.last_poll_duration_ns = poll_duration_ns;
                    if poll_duration_ns > call.max_poll_duration_ns {
                        call.max_poll_duration_ns = poll_duration_ns;
                    }
                }
                add_optional(&mut call.total_poll_alloc_bytes, poll_alloc_bytes);
                add_optional(&mut call.total_poll_alloc_count, poll_alloc_count);
                call.last_poll_alloc_bytes = poll_alloc_bytes;
                if let Some(poll_alloc_bytes) = poll_alloc_bytes {
                    if call
                        .max_poll_alloc_bytes
                        .is_none_or(|max| poll_alloc_bytes > max)
                    {
                        call.max_poll_alloc_bytes = Some(poll_alloc_bytes);
                    }
                }
                match result {
                    PollResult::Pending => {
                        update_future_state(call, FutureState::Suspended);
                    }
                    PollResult::Ready => {
                        update_future_state(call, FutureState::Ready);
                    }
                };
            }
        }
        FutureEvent::Completed {
            future_id,
            call_id,
            log_message,
        } => {
            let entry_logs = state
                .logs
                .entry(future_id)
                .or_insert_with(FutureEntryLogs::new);
            entry_logs.register_call(future_id, call_id);
            if let Some(call) = entry_logs.find_call_mut(call_id) {
                update_future_state(call, FutureState::Ready);
                call.result = log_message;
            }
        }
        FutureEvent::Cancelled { future_id, call_id } => {
            let entry_logs = state
                .logs
                .entry(future_id)
                .or_insert_with(FutureEntryLogs::new);
            entry_logs.register_call(future_id, call_id);
            if let Some(call) = entry_logs.find_call_mut(call_id) {
                update_future_state(call, FutureState::Cancelled);
            }
        }
    }
}

/// Trait for instrumenting futures (no Debug requirement).
///
/// This trait is not intended for direct use. Use the `future!` macro instead.
#[doc(hidden)]
pub trait InstrumentFuture {
    type Output;
    fn instrument_future(self, source: &'static str, label: Option<String>) -> Self::Output;
}

/// Trait for instrumenting futures with output logging (requires Debug).
///
/// This trait is not intended for direct use. Use the `future!` macro with `log = true` instead.
#[doc(hidden)]
pub trait InstrumentFutureLog {
    type Output;
    fn instrument_future_log(self, source: &'static str, label: Option<String>) -> Self::Output;
}

impl<F: std::future::Future> InstrumentFuture for F {
    type Output = InstrumentedFuture<F>;

    fn instrument_future(self, source: &'static str, label: Option<String>) -> Self::Output {
        InstrumentedFuture::new(self, source, label, None, true, None)
    }
}

impl<F: std::future::Future> InstrumentFutureLog for F
where
    F::Output: std::fmt::Debug,
{
    type Output = InstrumentedFutureLog<F>;

    fn instrument_future_log(self, source: &'static str, label: Option<String>) -> Self::Output {
        InstrumentedFutureLog::new(self, source, label, None, true, None)
    }
}

/// Custom labels come first (sorted alphabetically), then auto-generated labels (sorted by source).
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn compare_future_stats(a: &FutureEntry, b: &FutureEntry) -> std::cmp::Ordering {
    let a_has_label = a.label.is_some();
    let b_has_label = b.label.is_some();

    match (a_has_label, b_has_label) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (true, true) => a.label.as_ref().unwrap().cmp(b.label.as_ref().unwrap()),
        (false, false) => a.source.cmp(b.source),
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_sorted_future_stats() -> Vec<FutureEntry> {
    let Some(state) = FUTURES_STATE.get() else {
        return Vec::new();
    };
    let guard = state.inner.read().unwrap();
    let mut stats: Vec<FutureEntry> = guard.stats.values().cloned().collect();
    stats.sort_by(compare_future_stats);
    stats
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_futures_json() -> crate::json::JsonFuturesList {
    let data: Vec<JsonFutureEntry> = get_sorted_future_stats()
        .iter()
        .map(JsonFutureEntry::from)
        .collect();

    crate::json::JsonFuturesList {
        current_elapsed_ns: crate::lib_on::current_elapsed_ns(),
        total_count: data.len(),
        included_count: data.len(),
        data,
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_future_logs_list(future_id: u32) -> Option<FutureLogsList> {
    let state = FUTURES_STATE.get()?;
    let guard = state.inner.read().unwrap();
    let stats = guard.stats.get(&future_id)?;
    let entry_logs = guard.logs.get(&future_id)?;
    Some(FutureLogsList {
        id: future_id.to_string(),
        call_count: stats.logs_count,
        total_polls: stats.total_polls(),
        total_poll_duration_ns: stats.display_total_poll_duration_ns(),
        total_poll_alloc_bytes: stats.total_poll_alloc_bytes(),
        total_poll_alloc_count: stats.total_poll_alloc_count(),
        calls: entry_logs.logs.iter().rev().cloned().collect(),
    })
}

/// Instrument a future to inspect future's lifecycle events.
///
/// Optional parameters: `label`, `log = true`.
/// `log = true` requires `Debug` on the output type and prints `Ready(value)`.
///
/// # Examples
///
/// ```rust,ignore
/// use hotpath::future;
///
/// // Without logging (works with any output type)
/// let result = future!(async { NoDebugType::new() }).await;
///
/// // With logging (requires Debug on output type)
/// let result = future!(async { 42 }, log = true).await;
/// ```
#[macro_export]
macro_rules! future {
    ($fut:expr) => {{
        const FUTURE_LOC: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(FUTURE_LOC);
        $crate::futures::init_futures_state();
        $crate::InstrumentFuture::instrument_future($fut, FUTURE_LOC, None)
    }};

    // Literal labels are checked for uniqueness, then re-dispatched wrapped in
    // a block so they take the plain expression arms below.
    ($fut:expr, label = $label:literal) => {{
        $crate::__unique_label!(future, $label);
        $crate::future!($fut, label = { $label })
    }};
    ($fut:expr, label = $label:literal, log = true) => {{
        $crate::__unique_label!(future, $label);
        $crate::future!($fut, label = { $label }, log = true)
    }};
    ($fut:expr, log = true, label = $label:literal) => {{
        $crate::__unique_label!(future, $label);
        $crate::future!($fut, log = true, label = { $label })
    }};

    ($fut:expr, label = $label:expr) => {{
        const FUTURE_LOC: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(FUTURE_LOC);
        $crate::futures::init_futures_state();
        $crate::InstrumentFuture::instrument_future($fut, FUTURE_LOC, Some($label.to_string()))
    }};

    ($fut:expr, log = true) => {{
        const FUTURE_LOC: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(FUTURE_LOC);
        $crate::futures::init_futures_state();
        $crate::InstrumentFutureLog::instrument_future_log($fut, FUTURE_LOC, None)
    }};

    ($fut:expr, label = $label:expr, log = true) => {{
        const FUTURE_LOC: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(FUTURE_LOC);
        $crate::futures::init_futures_state();
        $crate::InstrumentFutureLog::instrument_future_log(
            $fut,
            FUTURE_LOC,
            Some($label.to_string()),
        )
    }};

    ($fut:expr, log = true, label = $label:expr) => {{
        const FUTURE_LOC: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(FUTURE_LOC);
        $crate::futures::init_futures_state();
        $crate::InstrumentFutureLog::instrument_future_log(
            $fut,
            FUTURE_LOC,
            Some($label.to_string()),
        )
    }};
}

#[cfg(test)]
mod tests {
    #[test]
    fn display_source_strips_column_from_call_site_ids() {
        let strip = crate::futures::display_source;
        assert_eq!(strip("src/main.rs:12:34"), "src/main.rs:12");
        assert_eq!(strip("my_module::my_future_fn"), "my_module::my_future_fn");
        assert_eq!(strip("app::handlers::fetch2"), "app::handlers::fetch2");
    }

    #[test]
    fn ready_future_state_is_terminal() {
        let mut call = crate::json::FutureLog::new(1, 1);
        crate::futures::update_future_state(&mut call, crate::futures::FutureState::Ready);

        crate::futures::update_future_state(&mut call, crate::futures::FutureState::Suspended);
        crate::futures::update_future_state(&mut call, crate::futures::FutureState::Cancelled);

        assert_eq!(call.state, crate::futures::FutureState::Ready);
    }

    #[test]
    fn cancelled_future_state_is_terminal() {
        let mut call = crate::json::FutureLog::new(1, 1);
        crate::futures::update_future_state(&mut call, crate::futures::FutureState::Cancelled);

        crate::futures::update_future_state(&mut call, crate::futures::FutureState::Suspended);
        crate::futures::update_future_state(&mut call, crate::futures::FutureState::Ready);

        assert_eq!(call.state, crate::futures::FutureState::Cancelled);
    }
}
