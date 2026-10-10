//! Function profiling module - measures execution time and memory allocations per function.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::{sync::LazyLock, sync::OnceLock, time::Duration};

use arc_swap::ArcSwapOption;
use crossbeam_channel::{bounded, Sender};

use crate::json::JsonFunctionsList;
use crate::output::FunctionLogsList;

#[cfg(feature = "hotpath-cpu")]
pub(crate) mod cpu;

pub(crate) mod allocator;
pub(crate) mod async_bridge;
pub(crate) mod exclusive;

pub(crate) use async_bridge::AsyncCallBridge;

cfg_if::cfg_if! {
    if #[cfg(feature = "hotpath-alloc")] {
        pub(crate) mod alloc;
        pub(crate) use alloc::state::{FunctionsState, Measurement};
        pub use alloc::guard::{MeasurementGuardAsync, MeasurementGuardSync};
        pub(crate) use alloc::guard::{MeasurementGuardAsyncWithLog, MeasurementGuardSyncWithLog};
    } else {
        pub(crate) mod timing;
        pub(crate) use timing::state::{FunctionsState, Measurement};
        pub use timing::guard::MeasurementGuard as MeasurementGuardAsync;
        pub use timing::guard::MeasurementGuard as MeasurementGuardSync;
        pub(crate) use timing::guard::MeasurementGuardWithLog as MeasurementGuardAsyncWithLog;
        pub(crate) use timing::guard::MeasurementGuardWithLog as MeasurementGuardSyncWithLog;
    }
}

pub(crate) struct FunctionStatsConfig {
    pub(crate) total_elapsed: Duration,
    pub(crate) percentiles: Vec<f64>,
    pub(crate) caller_name: &'static str,
    pub(crate) limit: usize,
    /// Set only for the final static report, never for the live metrics server.
    /// Cloud-report shape: values rendered with `Precision::Exact` (see
    /// `output.rs`).
    pub(crate) cloud: bool,
}

pub(crate) static FUNCTIONS_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

pub(crate) fn next_function_id() -> u32 {
    FUNCTIONS_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

enum Focus {
    Text(String),
    Regex(regex_lite::Regex),
}

static FOCUS_FILTER: LazyLock<Option<Focus>> = LazyLock::new(|| {
    let val = std::env::var("HOTPATH_FOCUS").ok()?;
    if let Some(pattern) = val.strip_prefix('/').and_then(|s| s.strip_suffix('/')) {
        let regex = regex_lite::Regex::new(pattern).unwrap_or_else(|err| {
            panic!(
                "Invalid HOTPATH_FOCUS regex pattern {pattern:?}: {err}\n\
                 Patterns use regex-lite syntax: \\b, \\w, \\d and \\s are ASCII-only \
                 and Unicode classes like \\p{{..}} are unsupported."
            )
        });
        Some(Focus::Regex(regex))
    } else {
        Some(Focus::Text(val))
    }
});

/// Compiles the `HOTPATH_FOCUS` filter so an invalid pattern fails at guard
/// construction instead of inside the first measured function.
pub(crate) fn init_focus_filter() {
    LazyLock::force(&FOCUS_FILTER);
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
#[inline]
fn is_focused(name: &str) -> bool {
    match &*FOCUS_FILTER {
        None => true,
        Some(Focus::Text(filter)) => name.contains(filter.as_str()),
        Some(Focus::Regex(re)) => re.is_match(name),
    }
}

pub(crate) static EXCLUDE_WRAPPER: LazyLock<bool> =
    LazyLock::new(|| crate::shared::env_flag("HOTPATH_EXCLUDE_WRAPPER"));

#[doc(hidden)]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
pub fn build_measurement_guard_sync(
    measurement_name: &'static str,
    wrapper: bool,
) -> MeasurementGuardSync {
    let skipped = !wrapper && !is_focused(measurement_name);
    MeasurementGuardSync::new_caller_scoped(measurement_name, wrapper, skipped)
}

/// Guard for `measure_block!`. Never caller-scoped: a block expression can
/// contain `.await` inside an async function, so its guard may live across
/// suspension points and threads where a thread-local caller scope would
/// leak to interleaved tasks. Only sync function bodies (which the compiler
/// guarantees are await-free) get caller scoping.
#[doc(hidden)]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
pub fn build_measurement_guard_block(
    measurement_name: &'static str,
    wrapper: bool,
) -> MeasurementGuardSync {
    let skipped = !wrapper && !is_focused(measurement_name);
    MeasurementGuardSync::new(measurement_name, wrapper, skipped)
}

/// Bridge between a measured async body's polls and its guard: carries the
/// per-poll allocation totals (`hotpath-alloc`) and exclusive times
/// (`HOTPATH_TIME_EXCLUSIVE`). `None` when neither is collected.
#[inline]
fn make_call_bridge(skipped: bool) -> Option<std::sync::Arc<AsyncCallBridge>> {
    let needed = cfg!(feature = "hotpath-alloc") || *exclusive::TIME_EXCLUSIVE;
    (needed && !skipped).then(|| std::sync::Arc::new(AsyncCallBridge::default()))
}

#[inline]
fn build_measurement_guard_async_with_bridge(
    measurement_name: &'static str,
    wrapper: bool,
) -> (
    MeasurementGuardAsync,
    Option<std::sync::Arc<AsyncCallBridge>>,
) {
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let skipped = !wrapper && !is_focused(measurement_name);
    let bridge = make_call_bridge(skipped);
    let guard =
        MeasurementGuardAsync::new_async(measurement_name, wrapper, skipped, bridge.clone());
    (guard, bridge)
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
fn build_measurement_guard_sync_with_log(
    measurement_name: &'static str,
    wrapper: bool,
) -> MeasurementGuardSyncWithLog {
    let skipped = !wrapper && !is_focused(measurement_name);
    MeasurementGuardSyncWithLog::new_caller_scoped(measurement_name, wrapper, skipped)
}

#[inline]
fn build_measurement_guard_async_with_log_bridge(
    measurement_name: &'static str,
    wrapper: bool,
) -> (
    MeasurementGuardAsyncWithLog,
    Option<std::sync::Arc<AsyncCallBridge>>,
) {
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let skipped = !wrapper && !is_focused(measurement_name);
    let bridge = make_call_bridge(skipped);
    let guard =
        MeasurementGuardAsyncWithLog::new_async(measurement_name, wrapper, skipped, bridge.clone());
    (guard, bridge)
}

/// Internal helper used by `#[hotpath::measure(log = true)]` for sync functions.
///
/// `measurement_loc` is the fully-qualified function path used as the metrics key.
/// `f` is the function body closure; its return value is recorded in recent logs.
#[doc(hidden)]
#[inline]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub fn measure_sync_log<T: std::fmt::Debug, F: FnOnce() -> T>(
    measurement_loc: &'static str,
    f: F,
) -> T {
    let guard = build_measurement_guard_sync_with_log(measurement_loc, false);
    let result = f();
    guard.finish_with_result(&result);
    result
}

/// Internal helper used by `#[hotpath::measure(log = true)]` for async functions.
///
/// `measurement_loc` is the fully-qualified function path used as the metrics key.
/// `fut` is the already-constructed async body future whose output is logged.
#[doc(hidden)]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub async fn measure_async_log<T: std::fmt::Debug, Fut>(
    measurement_loc: &'static str,
    fut: Fut,
) -> T
where
    Fut: Future<Output = T>,
{
    let (guard, bridge) = build_measurement_guard_async_with_log_bridge(measurement_loc, false);
    let result = await_measured(measurement_loc, fut, bridge).await;
    guard.finish_with_result(&result);
    result
}

/// Caller scope for a measured async body: `None` when `HOTPATH_FOCUS`
/// excludes the function, matching the sync guards' skipped behavior.
fn async_caller_scope(measurement_loc: &'static str) -> Option<&'static str> {
    is_focused(measurement_loc).then_some(measurement_loc)
}

/// Whether measured async bodies always need the poll wrapper: per-poll
/// allocation tracking, or caller scoping for SQL/HTTP source attribution.
const POLL_WRAPPER_ALWAYS: bool = cfg!(any(
    feature = "hotpath-alloc",
    feature = "sqlx",
    feature = "diesel",
    feature = "toasty",
    feature = "reqwest-0-12",
    feature = "reqwest-0-13",
    feature = "ureq-3",
));

/// Runs a measured async body through the poll wrapper, which scopes every
/// poll: caller stack entry (SQL/HTTP source attribution), allocation totals
/// and exclusive time, the latter two reported through `bridge`. Awaits the
/// future unwrapped when none of those is collected.
async fn await_measured<T, Fut>(
    measurement_loc: &'static str,
    fut: Fut,
    bridge: Option<std::sync::Arc<AsyncCallBridge>>,
) -> T
where
    Fut: Future<Output = T>,
{
    if POLL_WRAPPER_ALWAYS || bridge.is_some() {
        crate::futures::wrapper::InstrumentedFuture::new(
            fut,
            measurement_loc,
            None,
            bridge,
            false,
            async_caller_scope(measurement_loc),
        )
        .await
    } else {
        fut.await
    }
}

/// Internal helper used by `#[hotpath::measure]` for async functions.
///
/// `measurement_loc` is the fully-qualified function path used as the metrics key.
/// `fut` is the async body future measured for timing/allocs only.
#[doc(hidden)]
pub async fn measure_async<T, Fut>(measurement_loc: &'static str, fut: Fut) -> T
where
    Fut: Future<Output = T>,
{
    let (_guard, bridge) = build_measurement_guard_async_with_bridge(measurement_loc, false);
    await_measured(measurement_loc, fut, bridge).await
}

/// Internal helper used by `#[hotpath::measure(future = true)]`.
///
/// `measurement_loc` is the fully-qualified function path used for both function
/// measurement and visible future lifecycle events.
/// `fut` is the async body future to instrument.
#[doc(hidden)]
pub async fn measure_async_future<T, Fut>(measurement_loc: &'static str, fut: Fut) -> T
where
    Fut: Future<Output = T>,
{
    crate::futures::init_futures_state();

    let (_guard, bridge) = build_measurement_guard_async_with_bridge(measurement_loc, false);
    crate::futures::wrapper::InstrumentedFuture::new(
        fut,
        measurement_loc,
        None,
        bridge,
        true,
        async_caller_scope(measurement_loc),
    )
    .await
}

/// Internal helper used by `#[hotpath::measure(future = true, log = true)]`.
///
/// `measurement_loc` is the fully-qualified function path used for function metrics
/// and future lifecycle events.
/// `fut` is the async body future; its output is recorded in future/function logs.
#[doc(hidden)]
pub async fn measure_async_future_log<T, Fut>(measurement_loc: &'static str, fut: Fut) -> T
where
    T: std::fmt::Debug,
    Fut: Future<Output = T>,
{
    crate::futures::init_futures_state();

    let (guard, bridge) = build_measurement_guard_async_with_log_bridge(measurement_loc, false);
    let result = crate::futures::wrapper::InstrumentedFutureLog::new(
        fut,
        measurement_loc,
        None,
        bridge,
        true,
        async_caller_scope(measurement_loc),
    )
    .await;
    guard.finish_with_result(&result);
    result
}

pub(crate) static FUNCTIONS_STATE: OnceLock<
    ArcSwapOption<crate::lib_on::MetaRwLock<FunctionsState>>,
> = OnceLock::new();

pub(crate) static FUNCTIONS_QUERY_TX: OnceLock<crossbeam_channel::Sender<FunctionsQuery>> =
    OnceLock::new();

static CPU_LABEL_ALIASES: OnceLock<crate::lib_on::MetaRwLock<HashMap<&'static str, &'static str>>> =
    OnceLock::new();

#[doc(hidden)]
pub fn register_cpu_label_alias(label: &'static str, symbol: &'static str) {
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let map = CPU_LABEL_ALIASES
        .get_or_init(|| crate::lib_on::meta_rw_lock!("cpu_label_aliases", HashMap::new()));
    if let Ok(mut w) = map.write() {
        w.entry(label).or_insert(symbol);
    }
}

#[cfg(feature = "hotpath-cpu")]
pub(crate) fn get_cpu_label_aliases() -> HashMap<&'static str, &'static str> {
    CPU_LABEL_ALIASES
        .get()
        .and_then(|m| m.read().ok().map(|g| g.clone()))
        .unwrap_or_default()
}

/// Per-route slice of one function's measurements (calls made under an axum
/// route scope), running totals only - what the Prometheus
/// `hotpath_function_route_*` families export. The report stays per function.
#[derive(Debug, Default, Clone)]
#[cfg(feature = "hotpath-prometheus")]
pub(crate) struct RouteFunctionStats {
    pub(crate) count: u64,
    /// Calls that carried a duration (not skipped by time sampling).
    pub(crate) sampled_count: u64,
    pub(crate) total_duration_ns: u64,
    pub(crate) total_bytes: u64,
    pub(crate) total_allocs: u64,
}

#[cfg(feature = "hotpath-prometheus")]
pub(crate) type RouteStatsMap = HashMap<&'static str, RouteFunctionStats>;

#[cfg(feature = "hotpath-prometheus")]
impl RouteFunctionStats {
    #[inline]
    pub(crate) fn record(
        &mut self,
        duration_ns: Option<u64>,
        bytes_total: Option<u64>,
        count_total: Option<u64>,
    ) {
        self.count += 1;
        if let Some(duration_ns) = duration_ns {
            self.sampled_count += 1;
            self.total_duration_ns += duration_ns;
        }
        self.total_bytes += bytes_total.unwrap_or(0);
        self.total_allocs += count_total.unwrap_or(0);
    }
}

/// Raw per-route slice of a function for the Prometheus exporter, sorted by
/// route so the scrape output is stable.
#[cfg(feature = "hotpath-prometheus")]
#[derive(Debug, Clone)]
pub(crate) struct RawRouteFunction {
    pub(crate) route: &'static str,
    pub(crate) stats: RouteFunctionStats,
}

#[cfg(feature = "hotpath-prometheus")]
pub(crate) fn raw_routes(routes: &RouteStatsMap) -> Vec<RawRouteFunction> {
    let mut raw: Vec<RawRouteFunction> = routes
        .iter()
        .map(|(&route, stats)| RawRouteFunction {
            route,
            stats: stats.clone(),
        })
        .collect();
    raw.sort_by(|a, b| a.route.cmp(b.route));
    raw
}

/// Raw timing snapshot entry for the Prometheus exporter, with bucket
/// projections pre-computed worker-side so no histogram crosses the channel.
/// Classic counts come from the hdr histogram, not the coarser native
/// buckets, so boundary-adjacent observations stay in the correct `le` bucket.
/// `total_duration_ns` covers only sampled calls.
#[cfg(feature = "hotpath-prometheus")]
#[derive(Debug)]
pub(crate) struct RawFunctionTiming {
    pub(crate) name: &'static str,
    pub(crate) count: u64,
    pub(crate) sampled_count: u64,
    pub(crate) total_duration_ns: u64,
    pub(crate) native_buckets: Vec<(i32, u64)>,
    pub(crate) bucket_counts: Vec<u64>,
    pub(crate) routes: Vec<RawRouteFunction>,
}

/// Raw allocation snapshot entry for the Prometheus exporter - numeric fields
/// only, bucket projections pre-computed worker-side so no histogram crosses
/// the channel. `bytes_sample_count` / `allocs_sample_count` are the exact
/// histogram populations (`hist.len()`); both are 0 for `is_async` entries
/// whose measurements carried no per-call totals (the report's N/A
/// percentiles), which export only the running totals.
#[cfg(feature = "hotpath-prometheus")]
#[derive(Debug)]
pub(crate) struct RawFunctionAlloc {
    pub(crate) name: &'static str,
    pub(crate) total_bytes: u64,
    pub(crate) total_allocs: u64,
    pub(crate) bytes_sample_count: u64,
    pub(crate) allocs_sample_count: u64,
    pub(crate) bytes_native: Vec<(i32, u64)>,
    pub(crate) bytes_classic: Vec<u64>,
    pub(crate) allocs_native: Vec<(i32, u64)>,
    pub(crate) allocs_classic: Vec<u64>,
    /// Calls that allocated nothing - recorded as literal zeros, which the
    /// native format carries in zero_count rather than a log-scale bucket.
    pub(crate) bytes_zero_count: u64,
    pub(crate) allocs_zero_count: u64,
    pub(crate) routes: Vec<RawRouteFunction>,
}

/// Query request sent from TUI HTTP server to profiler worker thread
#[derive(Debug)]
pub(crate) enum FunctionsQuery {
    /// Request timing metrics snapshot
    Timing(Sender<JsonFunctionsList>),
    /// Request raw numeric timing snapshot (Prometheus exporter)
    #[cfg(feature = "hotpath-prometheus")]
    TimingRaw(Sender<Vec<RawFunctionTiming>>),
    /// Request raw numeric allocation snapshot (Prometheus exporter) - `None`
    /// if hotpath-alloc is not enabled
    #[cfg(feature = "hotpath-prometheus")]
    AllocRaw(Sender<Option<Vec<RawFunctionAlloc>>>),
    /// Request full metrics snapshot (allocation metrics) - returns None if hotpath-alloc not enabled
    Alloc(Sender<Option<JsonFunctionsList>>),
    /// Request the names + worker-assigned ids of functions that have been registered
    #[cfg(feature = "hotpath-cpu")]
    NamesAndIds(Sender<HashMap<&'static str, u32>>),
    /// Request timing function logs for a specific function by ID
    LogsTiming {
        function_id: u32,
        response_tx: Sender<Option<FunctionLogsList>>,
    },
    /// Request allocation function logs for a specific function by ID
    LogsAlloc {
        function_id: u32,
        response_tx: Sender<Option<FunctionLogsList>>,
    },
}

/// `None` when the worker has not started or did not answer within `timeout_ms`.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
fn query_functions_state<T, F>(timeout_ms: u64, make_query: F) -> Option<T>
where
    F: FnOnce(Sender<T>) -> FunctionsQuery,
{
    let query_tx = FUNCTIONS_QUERY_TX.get()?;
    let (response_tx, response_rx) = bounded::<T>(1);
    query_tx.send(make_query(response_tx)).ok()?;
    response_rx
        .recv_timeout(Duration::from_millis(timeout_ms))
        .ok()
}

fn query_json<T, F>(make_query: F) -> Option<T>
where
    F: FnOnce(Sender<T>) -> FunctionsQuery,
{
    query_functions_state(crate::metrics_server::RECV_TIMEOUT_MS, make_query)
}

/// `None`: worker unreachable or timed out (the metrics server answers 503).
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_functions_timing_json() -> Option<JsonFunctionsList> {
    query_json(FunctionsQuery::Timing)
}

#[cfg(feature = "hotpath-prometheus")]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_functions_raw() -> Option<Vec<RawFunctionTiming>> {
    query_functions_state(
        crate::prometheus_server::RECV_TIMEOUT_MS,
        FunctionsQuery::TimingRaw,
    )
}

/// Outer `None`: worker unreachable or timed out (the scrape must abort with
/// 503, never serve 200 with the alloc series missing); inner `None`:
/// hotpath-alloc not enabled.
#[cfg(feature = "hotpath-prometheus")]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_functions_alloc_raw() -> Option<Option<Vec<RawFunctionAlloc>>> {
    query_functions_state(
        crate::prometheus_server::RECV_TIMEOUT_MS,
        FunctionsQuery::AllocRaw,
    )
}

/// Outer `None`: worker unreachable or timed out; inner `None`: no function
/// with this id.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_function_logs_timing(function_id: u32) -> Option<Option<FunctionLogsList>> {
    query_json(|response_tx| FunctionsQuery::LogsTiming {
        function_id,
        response_tx,
    })
}

/// Outer `None`: worker unreachable or timed out; inner `None`: hotpath-alloc
/// not enabled.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_functions_alloc_json() -> Option<Option<JsonFunctionsList>> {
    query_json(FunctionsQuery::Alloc)
}

#[cfg(feature = "hotpath-cpu")]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_instrumented_names_and_ids() -> Option<HashMap<&'static str, u32>> {
    query_json(FunctionsQuery::NamesAndIds)
}

/// Outer `None`: worker unreachable or timed out; inner `None`: hotpath-alloc
/// not enabled or no function with this id.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_function_logs_alloc(function_id: u32) -> Option<Option<FunctionLogsList>> {
    query_json(|response_tx| FunctionsQuery::LogsAlloc {
        function_id,
        response_tx,
    })
}
