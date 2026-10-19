//! HTTP server instrumentation module - tracks response times per matched
//! route (`GET /users/{id}`).
//!
//! Entries are keyed by `METHOD route-template`, so the router's own path
//! template does the bucketing. Requests that never matched a route split on
//! their status: error responses collapse into a per-method `<unmatched>`
//! bucket (scanner probes like `GET /.env` would otherwise create one series
//! each), while successful ones (fallbacks, `nest_service` targets) keep the
//! raw path normalized by [`crate::lib_on::http::normalize`]. Distinct keys
//! are capped at `HOTPATH_ENTRIES_LIMIT`; beyond that new keys land in the
//! `<other>` bucket (see [`crate::lib_on::hotpath_guard::bounded_key`]).
//!
//! The write path (worker, events) is driven by the tower [`AxumLayer`]
//! (attached via the `axum!` macro or `Router::layer`), gated behind the
//! `axum-0-8` feature; the read path stays compiled so the report/metrics
//! wiring is feature-uniform.
#![cfg_attr(not(feature = "axum-0-8"), allow(dead_code))]

use crossbeam_channel::{bounded, Receiver as CbReceiver, RecvTimeoutError, Sender as CbSender};
use hdrhistogram::Histogram;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use crate::lib_on::caller_stack::{RequestAlloc, RequestCalls};
use crate::lib_on::{meta_rw_lock, MetaRwLock};

use crate::batch::{EventProducer, EventQueueRegistry};
use crate::instant::Instant;
use crate::json::{HttpLogEntry, HttpLogs};
use crate::lib_on::hotpath_guard::{
    bounded_key, DRAIN_INTERVAL_MS, LOGS_LIMIT, OVERFLOW_ENTRY, UNMATCHED_ENTRY,
};
use crate::lib_on::http::normalize::normalize_endpoint;
use crate::lib_on::START_TIME;
use crate::metrics_server::METRICS_SERVER_PORT;

#[cfg(feature = "axum-0-8")]
mod axum_08;

static SERVER_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

fn next_server_id() -> u32 {
    SERVER_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Tower layer that reports per-request response times to the hotpath server
/// worker, bucketed by the axum route template that handled the request.
///
/// The `axum!` macro appends it automatically; apps that build their own
/// middleware stack can add it with `Router::layer(hotpath::AxumLayer::new())`.
/// `Router::layer` only wraps routes added before it, so attach it after the
/// last `.route(..)` / `.fallback(..)` call.
#[derive(Clone, Copy, Debug)]
pub struct AxumLayer {
    _private: (),
}

impl AxumLayer {
    pub fn new() -> Self {
        init_server_state();
        Self { _private: () }
    }
}

impl Default for AxumLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Events sent to the background server statistics collection thread.
#[derive(Debug)]
pub(crate) enum ServerEvent {
    /// Emitted when the response head is produced. `route` is
    /// `METHOD template` when matched, else `METHOD raw-path` with `matched`
    /// false. `timestamp_ns` is ns since profiler start. `calls` / `alloc`
    /// cover the request's route scope, `None` when it had none (unmatched
    /// route, route scoping disabled, or the route interner cap was hit).
    Completed {
        route: Arc<str>,
        matched: bool,
        duration_nanos: u64,
        status: u16,
        timestamp_ns: u64,
        calls: Option<RequestCalls>,
        alloc: Option<RequestAlloc>,
    },
}

/// Aggregated statistics for a single route.
#[derive(Debug)]
pub(crate) struct ServerEntry {
    pub(crate) id: u32,
    pub(crate) route: String,
    pub(crate) count: u64,
    pub(crate) status_4xx: u64,
    pub(crate) status_5xx: u64,
    pub(crate) total_nanos: u64,
    /// Completed requests that carried a route scope; the denominator of
    /// the per-request SQL / HTTP averages.
    pub(crate) scoped_count: u64,
    /// SQL queries issued by scoped requests.
    pub(crate) sql_calls: u64,
    /// Outbound HTTP requests issued by scoped requests.
    pub(crate) http_calls: u64,
    /// Bytes allocated by scoped requests (`hotpath-alloc` only, else 0).
    pub(crate) alloc_bytes: u64,
    /// Allocations made by scoped requests (`hotpath-alloc` only, else 0).
    pub(crate) alloc_count: u64,
    hist: Option<Histogram<u64>>,
    /// Bytes allocated per scoped request; same bounds as the function alloc
    /// histograms so the exported buckets line up with
    /// `hotpath_function_alloc_bytes`. `None` without `hotpath-alloc`, where
    /// every request would record a zero into ~170 KB of buckets per route.
    bytes_hist: Option<Histogram<u64>>,
}

fn per_request(calls: u64, scoped_count: u64) -> Option<f64> {
    (scoped_count > 0).then(|| calls as f64 / scoped_count as f64)
}

impl ServerEntry {
    const LOW_NS: u64 = 1;
    const HIGH_NS: u64 = crate::lib_on::MAX_DURATION_NS;
    const LOW_BYTES: u64 = 1;
    const HIGH_BYTES: u64 = 1_000_000_000; // 1GB
    const SIGFIGS: u8 = 3;

    fn new(id: u32, route: String) -> Self {
        Self {
            id,
            route,
            count: 0,
            status_4xx: 0,
            status_5xx: 0,
            total_nanos: 0,
            scoped_count: 0,
            sql_calls: 0,
            http_calls: 0,
            alloc_bytes: 0,
            alloc_count: 0,
            hist: Histogram::<u64>::new_with_bounds(Self::LOW_NS, Self::HIGH_NS, Self::SIGFIGS)
                .ok(),
            bytes_hist: cfg!(feature = "hotpath-alloc")
                .then(|| {
                    Histogram::<u64>::new_with_bounds(
                        Self::LOW_BYTES,
                        Self::HIGH_BYTES,
                        Self::SIGFIGS,
                    )
                    .ok()
                })
                .flatten(),
        }
    }

    #[inline]
    fn record(&mut self, nanos: u64) {
        if let Some(ref mut hist) = self.hist {
            hist.record(nanos.clamp(Self::LOW_NS, Self::HIGH_NS))
                .unwrap();
        }
    }

    /// Zero is always recordable regardless of the histogram's lowest
    /// discernible value, so requests that allocated nothing count toward
    /// the percentiles.
    #[inline]
    fn record_alloc(&mut self, alloc: RequestAlloc) {
        self.alloc_bytes += alloc.bytes;
        self.alloc_count += alloc.count;
        if let Some(ref mut hist) = self.bytes_hist {
            hist.record(alloc.bytes.min(Self::HIGH_BYTES)).unwrap();
        }
    }

    /// Average bytes allocated per scoped request, `None` when no completed
    /// request of this route carried a route scope.
    pub(crate) fn bytes_per_request(&self) -> Option<f64> {
        per_request(self.alloc_bytes, self.scoped_count)
    }

    /// Average allocations per scoped request, `None` when no completed
    /// request of this route carried a route scope.
    pub(crate) fn allocs_per_request(&self) -> Option<f64> {
        per_request(self.alloc_count, self.scoped_count)
    }

    pub(crate) fn avg_bytes(&self) -> u64 {
        self.alloc_bytes.checked_div(self.scoped_count).unwrap_or(0)
    }

    pub(crate) fn percentile_bytes(&self, p: f64) -> u64 {
        match self.bytes_hist {
            Some(ref hist) if self.scoped_count > 0 => {
                hist.value_at_percentile(p.clamp(0.0, 100.0))
            }
            _ => 0,
        }
    }

    /// Sparse native-histogram buckets of bytes per scoped request.
    #[cfg(all(feature = "hotpath-prometheus", feature = "hotpath-alloc"))]
    pub(crate) fn alloc_native_buckets(&self, schema: i32) -> Vec<(i32, u64)> {
        crate::lib_on::native_histograms::native_buckets_opt(
            self.bytes_hist.as_ref(),
            self.scoped_count > 0,
            schema,
            crate::lib_on::native_histograms::UNIT_SCALE,
        )
    }

    /// Cumulative classic-bucket counts of bytes per scoped request at or
    /// below each boundary.
    #[cfg(all(feature = "hotpath-prometheus", feature = "hotpath-alloc"))]
    pub(crate) fn alloc_classic_buckets(&self, boundaries: &[u64]) -> Vec<u64> {
        crate::lib_on::native_histograms::classic_buckets_opt(
            self.bytes_hist.as_ref(),
            self.scoped_count > 0,
            boundaries,
        )
    }

    /// Scoped requests that allocated nothing (native `zero_count`).
    #[cfg(all(feature = "hotpath-prometheus", feature = "hotpath-alloc"))]
    pub(crate) fn alloc_zero_count(&self) -> u64 {
        self.bytes_hist.as_ref().map_or(0, |h| h.count_at(0))
    }

    /// Average SQL queries per scoped request, `None` when no completed
    /// request of this route carried a route scope.
    pub(crate) fn sql_per_request(&self) -> Option<f64> {
        per_request(self.sql_calls, self.scoped_count)
    }

    /// Average outbound HTTP requests per scoped request, `None` when no
    /// completed request of this route carried a route scope.
    pub(crate) fn http_per_request(&self) -> Option<f64> {
        per_request(self.http_calls, self.scoped_count)
    }

    pub(crate) fn avg_nanos(&self) -> u64 {
        self.total_nanos.checked_div(self.count).unwrap_or(0)
    }

    pub(crate) fn percentile_nanos(&self, p: f64) -> u64 {
        match self.hist {
            Some(ref hist) if self.count > 0 => hist.value_at_percentile(p.clamp(0.0, 100.0)),
            _ => 0,
        }
    }

    /// Sparse native-histogram buckets of recorded durations, `(index, count)`
    /// at `schema`, for the Prometheus exporter.
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn native_buckets(&self, schema: i32) -> Vec<(i32, u64)> {
        match self.hist.as_ref().filter(|_| self.count > 0) {
            Some(hist) => crate::lib_on::native_histograms::native_bucket_counts(
                hist,
                schema,
                crate::lib_on::native_histograms::NANOS_SCALE,
            ),
            None => Vec::new(),
        }
    }

    /// Cumulative classic-bucket counts of recorded durations at or below each
    /// boundary (ns), exact to the histogram's 0.1% resolution.
    #[cfg(feature = "hotpath-prometheus")]
    pub(crate) fn classic_buckets(&self, boundaries: &[u64]) -> Vec<u64> {
        match self.hist.as_ref().filter(|_| self.count > 0) {
            Some(hist) => {
                crate::lib_on::native_histograms::cumulative_bucket_counts(hist, boundaries)
            }
            None => vec![0; boundaries.len()],
        }
    }
}

#[derive(Default)]
pub(crate) struct ServerInternalState {
    pub(crate) stats: HashMap<String, ServerEntry>,
    /// Recent requests per entry id, capped at `LOGS_LIMIT`. Only status and
    /// timing are kept - raw request paths are never stored.
    pub(crate) logs: HashMap<u32, VecDeque<HttpLogEntry>>,
}

pub(crate) struct ServerState {
    pub(crate) inner: Arc<MetaRwLock<ServerInternalState>>,
    pub(crate) shutdown_tx: StdMutex<Option<CbSender<()>>>,
    pub(crate) completion_rx: StdMutex<Option<CbReceiver<()>>>,
}

pub(crate) static SERVER_STATE: OnceLock<ServerState> = OnceLock::new();

/// Runs `f` on the entries sorted for display, borrowed under the read lock:
/// nothing is cloned, so the per-entry histograms stay in the map.
pub(crate) fn with_sorted_server_entries<R>(f: impl FnOnce(&[&ServerEntry]) -> R) -> R {
    let Some(state) = SERVER_STATE.get() else {
        return f(&[]);
    };
    let guard = state.inner.read().unwrap();
    let mut stats: Vec<&ServerEntry> = guard.stats.values().collect();
    stats.sort_by(|a, b| compare_server_entries(a, b));
    f(&stats)
}

/// Returns recent requests of the route entry with the given id, newest first.
pub(crate) fn get_server_logs(id: u32) -> Option<HttpLogs> {
    let state = SERVER_STATE.get()?;
    let guard = state.inner.read().unwrap();
    let logs = guard.logs.get(&id)?;
    Some(HttpLogs {
        id,
        logs: logs.iter().rev().cloned().collect(),
    })
}

pub(crate) fn get_server_json() -> crate::json::JsonServerList {
    let elapsed = std::time::Duration::from_nanos(crate::lib_on::current_elapsed_ns());
    let percentiles = crate::lib_on::hotpath_guard::configured_percentiles();
    let columns = crate::lib_on::report::ServerColumns::from_state();
    with_sorted_server_entries(|entries| {
        crate::lib_on::report::collect_server_json(
            entries,
            0,
            elapsed,
            &percentiles,
            columns,
            false,
        )
    })
}

static EVENT_QUEUES: EventQueueRegistry<ServerEvent> = EventQueueRegistry::new();

thread_local! {
    static EVENT_PRODUCER: EventProducer<ServerEvent> = EVENT_QUEUES.register();
}

#[inline]
pub(crate) fn send_server_event(event: ServerEvent) {
    if !EVENT_QUEUES.is_active() {
        return;
    }
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let _ = EVENT_PRODUCER.try_with(|producer| producer.push(event));
}

/// Stops producers ahead of the worker's final sweep at shutdown.
pub(crate) fn stop_server_events() {
    EVENT_QUEUES.set_active(false);
    #[cfg(feature = "axum-0-8")]
    crate::lib_on::caller_stack::set_route_scope(false);
}

/// Bucket key for an unmatched request that ended in an error status: the
/// method prefix of the `METHOD path` route key is kept, the path is replaced
/// by [`UNMATCHED_ENTRY`] (`GET /.env` -> `GET <unmatched>`).
fn unmatched_key(route: &str) -> String {
    match route.split_once(' ') {
        Some((method, _path)) => format!("{method} {UNMATCHED_ENTRY}"),
        None => UNMATCHED_ENTRY.to_string(),
    }
}

fn process_server_event(state: &mut ServerInternalState, event: ServerEvent) {
    let ServerEvent::Completed {
        route,
        matched,
        duration_nanos,
        status,
        timestamp_ns,
        calls,
        alloc,
    } = event;

    // Scanner spam is by definition unmatched + error status; collapsing only
    // that combination keeps per-page stats for legit fallback-served 2xx/3xx
    // pages (docs `ServeDir`, `nest_service` blog posts) while making probe
    // volume a single visible series. The status is only known at completion,
    // which is exactly where this worker runs.
    let key = if matched {
        route.to_string()
    } else if status >= 400 {
        unmatched_key(&route)
    } else {
        normalize_endpoint(&route)
    };
    let key = bounded_key(&state.stats, key, || OVERFLOW_ENTRY.to_string());
    let entry = state
        .stats
        .entry(key)
        .or_insert_with_key(|route| ServerEntry::new(next_server_id(), route.clone()));
    entry.count += 1;
    entry.total_nanos += duration_nanos;
    match status {
        400..=499 => entry.status_4xx += 1,
        500..=599 => entry.status_5xx += 1,
        _ => {}
    }
    entry.record(duration_nanos);
    if let Some(calls) = calls {
        entry.scoped_count += 1;
        entry.sql_calls += u64::from(calls.sql);
        entry.http_calls += u64::from(calls.http);
        entry.record_alloc(alloc.unwrap_or_default());
    }

    let logs = state.logs.entry(entry.id).or_default();
    if logs.len() >= *LOGS_LIMIT {
        logs.pop_front();
    }
    logs.push_back(HttpLogEntry {
        index: entry.count,
        timestamp: timestamp_ns,
        duration_nanos,
        status: Some(status),
    });
}

fn flush_server_buffer(
    buffer: &mut Vec<ServerEvent>,
    inner: &Arc<MetaRwLock<ServerInternalState>>,
) {
    if buffer.is_empty() {
        return;
    }
    if let Ok(mut shared) = inner.write() {
        for e in buffer.drain(..) {
            process_server_event(&mut shared, e);
        }
    }
}

/// Initialize the server statistics collection system (called by the `axum!`
/// macro and the [`AxumLayer`] constructor).
pub(crate) fn init_server_state() {
    SERVER_STATE.get_or_init(|| {
        START_TIME.get_or_init(Instant::now);

        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let (completion_tx, completion_rx) = bounded::<()>(1);

        let inner = Arc::new(meta_rw_lock!(
            "server_state",
            ServerInternalState {
                stats: HashMap::new(),
                logs: HashMap::new(),
            },
        ));
        let inner_clone = Arc::clone(&inner);

        EVENT_QUEUES.set_active(true);

        crate::lib_on::spawn_profiler_thread("hp-server", move || {
            let flush_interval = std::time::Duration::from_millis(*DRAIN_INTERVAL_MS);
            let mut swept: Vec<ServerEvent> = Vec::new();

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
                    flush_server_buffer(&mut swept, &inner_clone);
                    break;
                }

                EVENT_QUEUES.sweep(&mut swept);
                flush_server_buffer(&mut swept, &inner_clone);
            }

            let _ = completion_tx.send(());
        })
        .expect("Failed to spawn server-stats-collector thread");

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        ServerState {
            inner,
            shutdown_tx: StdMutex::new(Some(shutdown_tx)),
            completion_rx: StdMutex::new(Some(completion_rx)),
        }
    });
}

/// Sort entries by total time spent (slowest aggregate first), tiebreak by count.
pub(crate) fn compare_server_entries(a: &ServerEntry, b: &ServerEntry) -> std::cmp::Ordering {
    b.total_nanos
        .cmp(&a.total_nanos)
        .then_with(|| b.count.cmp(&a.count))
        .then_with(|| a.id.cmp(&b.id))
}

/// Wrap an axum `Router` so every request it serves is timed and reported in
/// the `server` section, keyed by matched route (`GET /users/{id}`).
///
/// Expands to `router.layer(hotpath::AxumLayer::new())`, so it must be applied
/// after the last `.route(..)` / `.fallback(..)` call - `Router::layer` only
/// wraps routes that already exist. With the `hotpath` feature disabled the
/// router is returned unchanged.
///
/// # Examples
///
/// ```rust,ignore
/// let app = hotpath::axum!(Router::new()
///     .route("/users/{id}", get(get_user))
///     .route("/users", post(create_user)));
///
/// axum::serve(listener, app).await?;
/// ```
#[macro_export]
macro_rules! axum {
    ($router:expr) => {
        $router.layer($crate::AxumLayer::new())
    };
}

#[cfg(test)]
mod tests {
    use crate::lib_on::caller_stack::{RequestAlloc, RequestCalls};
    use crate::lib_on::server::{process_server_event, ServerEvent, ServerInternalState};
    use std::sync::Arc;

    fn completed(route: &str, calls: Option<RequestCalls>) -> ServerEvent {
        completed_alloc(route, calls, calls.map(|_| RequestAlloc::ZERO))
    }

    fn completed_alloc(
        route: &str,
        calls: Option<RequestCalls>,
        alloc: Option<RequestAlloc>,
    ) -> ServerEvent {
        ServerEvent::Completed {
            route: Arc::from(route),
            matched: true,
            duration_nanos: 1_000,
            status: 200,
            timestamp_ns: 0,
            calls,
            alloc,
        }
    }

    #[test]
    fn per_request_averages_use_scoped_requests_only() {
        let mut state = ServerInternalState::default();
        let route = "GET /users/{id}";
        process_server_event(
            &mut state,
            completed_alloc(
                route,
                Some(RequestCalls { sql: 3, http: 1 }),
                Some(RequestAlloc {
                    bytes: 3_000,
                    count: 30,
                }),
            ),
        );
        process_server_event(
            &mut state,
            completed_alloc(
                route,
                Some(RequestCalls { sql: 1, http: 0 }),
                Some(RequestAlloc {
                    bytes: 1_000,
                    count: 10,
                }),
            ),
        );
        // A request that lost its scope (interner cap) counts as a request
        // but not towards the averages.
        process_server_event(&mut state, completed(route, None));

        let entry = &state.stats[route];
        assert_eq!(entry.count, 3);
        assert_eq!(entry.scoped_count, 2);
        assert_eq!(entry.sql_per_request(), Some(2.0));
        assert_eq!(entry.http_per_request(), Some(0.5));
        assert_eq!(entry.alloc_bytes, 4_000);
        assert_eq!(entry.alloc_count, 40);
        assert_eq!(entry.bytes_per_request(), Some(2_000.0));
        assert_eq!(entry.allocs_per_request(), Some(20.0));
        assert_eq!(entry.avg_bytes(), 2_000);
        // hdrhistogram reports the highest equivalent value of its 0.1% bin;
        // without the counting allocator there is no histogram at all.
        if cfg!(feature = "hotpath-alloc") {
            assert!((3_000..=3_003).contains(&entry.percentile_bytes(100.0)));
        }

        process_server_event(&mut state, completed("GET /missing", None));
        let unscoped = &state.stats["GET /missing"];
        assert_eq!(unscoped.sql_per_request(), None);
        assert_eq!(unscoped.http_per_request(), None);
        assert_eq!(unscoped.bytes_per_request(), None);
        assert_eq!(unscoped.allocs_per_request(), None);
        assert_eq!(unscoped.percentile_bytes(50.0), 0);
    }

    #[cfg(feature = "hotpath-alloc")]
    #[test]
    fn zero_byte_requests_count_toward_alloc_percentiles() {
        let mut state = ServerInternalState::default();
        let route = "GET /static";
        for bytes in [0, 0, 0, 4_000] {
            process_server_event(
                &mut state,
                completed_alloc(
                    route,
                    Some(RequestCalls::ZERO),
                    Some(RequestAlloc { bytes, count: 0 }),
                ),
            );
        }
        let entry = &state.stats[route];
        assert_eq!(entry.scoped_count, 4);
        assert_eq!(entry.percentile_bytes(50.0), 0);
        assert!((4_000..=4_004).contains(&entry.percentile_bytes(100.0)));
    }

    fn unmatched(route: &str, status: u16) -> ServerEvent {
        ServerEvent::Completed {
            route: Arc::from(route),
            matched: false,
            duration_nanos: 1_000,
            status,
            timestamp_ns: 0,
            calls: None,
            alloc: None,
        }
    }

    #[test]
    fn unmatched_error_requests_collapse_per_method() {
        let mut state = ServerInternalState::default();
        process_server_event(&mut state, unmatched("GET /.env", 404));
        process_server_event(&mut state, unmatched("GET /wp-login.php", 404));
        process_server_event(&mut state, unmatched("POST /xmlrpc.php", 500));

        assert_eq!(state.stats.len(), 2);
        let get_bucket = &state.stats["GET <unmatched>"];
        assert_eq!(get_bucket.count, 2);
        assert_eq!(get_bucket.status_4xx, 2);
        let post_bucket = &state.stats["POST <unmatched>"];
        assert_eq!(post_bucket.count, 1);
        assert_eq!(post_bucket.status_5xx, 1);
    }

    #[test]
    fn unmatched_success_requests_keep_normalized_path() {
        let mut state = ServerInternalState::default();
        process_server_event(&mut state, unmatched("GET /blog/1234", 200));
        process_server_event(&mut state, unmatched("GET /pages/about", 301));

        let blog = &state.stats["GET /blog/{id}"];
        assert_eq!(blog.count, 1);
        let about = &state.stats["GET /pages/about"];
        assert_eq!(about.count, 1);
        assert!(!state.stats.keys().any(|k| k.contains("<unmatched>")));
    }
}
