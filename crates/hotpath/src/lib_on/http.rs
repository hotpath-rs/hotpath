//! HTTP request instrumentation module - tracks request durations per endpoint.
//!
//! Entries are keyed by *normalized* endpoint (`GET api.example.com/users/{id}`),
//! so parameter-varied requests to the same route merge into a single bucket
//! (see `normalize`). Normalization runs on the background worker thread to
//! keep the request path light.
//!
//! The write path (worker, events) is driven by the client middleware
//! front-ends ([`ReqwestHttpMiddleware`] / [`UreqHttpMiddleware`] attached via
//! the `http!` macro or manually), gated behind the `reqwest-0-12` /
//! `reqwest-0-13` / `ureq-3` features; the read path stays compiled so the
//! report/metrics wiring is feature-uniform.
#![cfg_attr(
    not(any(feature = "reqwest-0-12", feature = "reqwest-0-13", feature = "ureq-3")),
    allow(dead_code)
)]

use crossbeam_channel::{bounded, Receiver as CbReceiver, RecvTimeoutError, Sender as CbSender};
use hdrhistogram::Histogram;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use crate::lib_on::{meta_rw_lock, MetaRwLock};

use crate::batch::{EventProducer, EventQueueRegistry};
use crate::instant::Instant;
use crate::json::{HttpLogEntry, HttpLogs};
use crate::lib_on::hotpath_guard::{bounded_key, DRAIN_INTERVAL_MS, LOGS_LIMIT, OVERFLOW_ENTRY};
use crate::lib_on::START_TIME;
use crate::metrics_server::METRICS_SERVER_PORT;

pub(crate) mod normalize;
#[cfg(feature = "reqwest-0-12")]
mod reqwest_012;
#[cfg(feature = "reqwest-0-13")]
mod reqwest_013;
#[cfg(feature = "ureq-3")]
mod ureq_3;

static HTTP_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

fn next_http_id() -> u32 {
    HTTP_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Middleware that reports per-request timing to the hotpath HTTP worker.
///
/// The `http!` macro attaches it automatically; apps that already use
/// `reqwest-middleware` can attach it to their stack manually with `.with(...)`.
/// Stack order matters: placed inside a retry middleware it times each attempt
/// separately, outside it times the total including retries.
#[cfg_attr(
    not(any(feature = "reqwest-0-12", feature = "reqwest-0-13")),
    allow(dead_code)
)]
#[derive(Default, Clone)]
pub struct ReqwestHttpMiddleware {
    pub(crate) label: Option<String>,
}

impl ReqwestHttpMiddleware {
    pub fn new() -> Self {
        init_http_state();
        Self::default()
    }

    /// Prefixes every endpoint key produced by this middleware with `label`.
    pub fn with_label(label: impl Into<String>) -> Self {
        init_http_state();
        Self {
            label: Some(label.into()),
        }
    }
}

/// Middleware that reports per-request timing to the hotpath HTTP worker.
///
/// The `http!` macro appends it to a `ureq` `ConfigBuilder`; apps that want to
/// control its position in their middleware chain can add it manually with
/// `.middleware(hotpath::UreqHttpMiddleware::new())`. Chain order matters:
/// placed after a retry middleware it times each attempt separately, before it
/// times the total including retries.
#[cfg_attr(not(feature = "ureq-3"), allow(dead_code))]
#[derive(Default, Clone)]
pub struct UreqHttpMiddleware {
    pub(crate) label: Option<String>,
}

impl UreqHttpMiddleware {
    pub fn new() -> Self {
        init_http_state();
        Self::default()
    }

    /// Prefixes every endpoint key produced by this middleware with `label`.
    pub fn with_label(label: impl Into<String>) -> Self {
        init_http_state();
        Self {
            label: Some(label.into()),
        }
    }
}

/// Wraps an HTTP client so every request it sends is timed and reported.
/// Implemented for `reqwest::Client` (per enabled reqwest generation), where
/// the concrete client type routes to the matching `reqwest-middleware`
/// wrapper, and for ureq's `ConfigBuilder<AgentScope>`, which is returned with
/// the middleware appended.
pub trait InstrumentHttpClient {
    type Output;

    fn instrument_http(self, label: Option<String>) -> Self::Output;
}

/// Events sent to the background HTTP statistics collection thread.
#[derive(Debug)]
pub(crate) enum HttpEvent {
    /// Emitted when a request completes (response headers received or the
    /// request failed). `endpoint` is the raw `METHOD host/path` pre-key; the
    /// worker normalizes it to derive the bucket key. `status` is `None` for
    /// transport errors. `timestamp_ns` is the completion time in ns since
    /// profiler start. `source` is the innermost instrumented function on the
    /// caller stack when the request was issued, `None` when it was sent
    /// outside any measured scope. `route` is the axum route template handling
    /// the request that issued it, `None` outside the server middleware or with
    /// route scoping off.
    Executed {
        endpoint: Arc<str>,
        label: Option<Arc<str>>,
        duration_nanos: u64,
        status: Option<u16>,
        timestamp_ns: u64,
        source: Option<&'static str>,
        route: Option<&'static str>,
    },
}

/// `(route, source, normalized endpoint)`.
pub(crate) type HttpKey = (Option<&'static str>, Option<&'static str>, String);

/// Aggregated statistics for a single normalized endpoint requested from a
/// single source function under a single axum route. The same endpoint hit
/// from two instrumented functions (or under two routes) produces two entries.
#[derive(Debug)]
pub(crate) struct HttpEntry {
    pub(crate) id: u32,
    pub(crate) endpoint: String,
    pub(crate) source: Option<&'static str>,
    pub(crate) route: Option<&'static str>,
    pub(crate) count: u64,
    /// Transport errors plus responses with status >= 400.
    pub(crate) error_count: u64,
    pub(crate) total_nanos: u64,
    hist: Option<Histogram<u64>>,
}

impl HttpEntry {
    const LOW_NS: u64 = 1;
    const HIGH_NS: u64 = crate::lib_on::MAX_DURATION_NS;
    const SIGFIGS: u8 = 3;

    fn new(
        id: u32,
        endpoint: String,
        source: Option<&'static str>,
        route: Option<&'static str>,
    ) -> Self {
        Self {
            id,
            endpoint,
            source,
            route,
            count: 0,
            error_count: 0,
            total_nanos: 0,
            hist: Histogram::<u64>::new_with_bounds(Self::LOW_NS, Self::HIGH_NS, Self::SIGFIGS)
                .ok(),
        }
    }

    #[inline]
    fn record(&mut self, nanos: u64) {
        if let Some(ref mut hist) = self.hist {
            hist.record(nanos.clamp(Self::LOW_NS, Self::HIGH_NS))
                .unwrap();
        }
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

pub(crate) struct HttpInternalState {
    pub(crate) stats: HashMap<HttpKey, HttpEntry>,
    /// Recent requests per entry id, capped at `LOGS_LIMIT`. Log entries keep
    /// only status and timing - raw URLs (which could carry query params or
    /// path ids) are never stored.
    pub(crate) logs: HashMap<u32, VecDeque<HttpLogEntry>>,
}

pub(crate) struct HttpState {
    pub(crate) inner: Arc<MetaRwLock<HttpInternalState>>,
    pub(crate) shutdown_tx: StdMutex<Option<CbSender<()>>>,
    pub(crate) completion_rx: StdMutex<Option<CbReceiver<()>>>,
}

pub(crate) static HTTP_STATE: OnceLock<HttpState> = OnceLock::new();

/// Runs `f` on the entries sorted for display, borrowed under the read lock:
/// nothing is cloned, so the per-entry histograms stay in the map.
pub(crate) fn with_sorted_http_entries<R>(f: impl FnOnce(&[&HttpEntry]) -> R) -> R {
    let Some(state) = HTTP_STATE.get() else {
        return f(&[]);
    };
    let guard = state.inner.read().unwrap();
    let mut stats: Vec<&HttpEntry> = guard.stats.values().collect();
    stats.sort_by(|a, b| compare_http_entries(a, b));
    f(&stats)
}

/// Returns recent requests of the entry with the given id, newest first.
pub(crate) fn get_http_logs(id: u32) -> Option<HttpLogs> {
    let state = HTTP_STATE.get()?;
    let guard = state.inner.read().unwrap();
    let logs = guard.logs.get(&id)?;
    Some(HttpLogs {
        id,
        logs: logs.iter().rev().cloned().collect(),
    })
}

pub(crate) fn get_http_json() -> crate::json::JsonHttpList {
    let elapsed = std::time::Duration::from_nanos(crate::lib_on::current_elapsed_ns());
    let percentiles = crate::lib_on::hotpath_guard::configured_percentiles();
    with_sorted_http_entries(|entries| {
        crate::lib_on::report::collect_http_json(entries, 0, elapsed, &percentiles, false)
    })
}

static EVENT_QUEUES: EventQueueRegistry<HttpEvent> = EventQueueRegistry::new();

thread_local! {
    static EVENT_PRODUCER: EventProducer<HttpEvent> = EVENT_QUEUES.register();
}

#[inline]
pub(crate) fn send_http_event(event: HttpEvent) {
    if !EVENT_QUEUES.is_active() {
        return;
    }
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let _ = EVENT_PRODUCER.try_with(|producer| producer.push(event));
}

/// Stops producers ahead of the worker's final sweep at shutdown.
pub(crate) fn stop_http_events() {
    EVENT_QUEUES.set_active(false);
}

fn process_http_event(state: &mut HttpInternalState, event: HttpEvent) {
    let HttpEvent::Executed {
        endpoint,
        label,
        duration_nanos,
        status,
        timestamp_ns,
        source,
        route,
    } = event;

    let mut key = normalize::normalize_endpoint(&endpoint);
    if let Some(label) = label {
        key = format!("{label}: {key}");
    }
    let key = bounded_key(&state.stats, (route, source, key), || {
        (None, None, OVERFLOW_ENTRY.to_string())
    });
    let entry = state
        .stats
        .entry(key)
        .or_insert_with_key(|(route, source, endpoint)| {
            HttpEntry::new(next_http_id(), endpoint.clone(), *source, *route)
        });
    entry.count += 1;
    entry.total_nanos += duration_nanos;
    if status.is_none() || status.is_some_and(|s| s >= 400) {
        entry.error_count += 1;
    }
    entry.record(duration_nanos);

    let logs = state.logs.entry(entry.id).or_default();
    if logs.len() >= *LOGS_LIMIT {
        logs.pop_front();
    }
    logs.push_back(HttpLogEntry {
        index: entry.count,
        timestamp: timestamp_ns,
        duration_nanos,
        status,
    });
}

fn flush_http_buffer(buffer: &mut Vec<HttpEvent>, inner: &Arc<MetaRwLock<HttpInternalState>>) {
    if buffer.is_empty() {
        return;
    }
    if let Ok(mut shared) = inner.write() {
        for e in buffer.drain(..) {
            process_http_event(&mut shared, e);
        }
    }
}

/// Initialize the HTTP statistics collection system (called by the `http!`
/// macro and the [`ReqwestHttpMiddleware`] / [`UreqHttpMiddleware`]
/// constructors).
pub fn init_http_state() {
    HTTP_STATE.get_or_init(|| {
        START_TIME.get_or_init(Instant::now);

        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let (completion_tx, completion_rx) = bounded::<()>(1);

        let inner = Arc::new(meta_rw_lock!(
            "http_state",
            HttpInternalState {
                stats: HashMap::new(),
                logs: HashMap::new(),
            },
        ));
        let inner_clone = Arc::clone(&inner);

        EVENT_QUEUES.set_active(true);

        crate::lib_on::spawn_profiler_thread("hp-http", move || {
            let flush_interval = std::time::Duration::from_millis(*DRAIN_INTERVAL_MS);
            let mut swept: Vec<HttpEvent> = Vec::new();

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
                    flush_http_buffer(&mut swept, &inner_clone);
                    break;
                }

                EVENT_QUEUES.sweep(&mut swept);
                flush_http_buffer(&mut swept, &inner_clone);
            }

            let _ = completion_tx.send(());
        })
        .expect("Failed to spawn http-stats-collector thread");

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        HttpState {
            inner,
            shutdown_tx: StdMutex::new(Some(shutdown_tx)),
            completion_rx: StdMutex::new(Some(completion_rx)),
        }
    });
}

/// Sort entries by total time spent (slowest aggregate first), tiebreak by count.
pub(crate) fn compare_http_entries(a: &HttpEntry, b: &HttpEntry) -> std::cmp::Ordering {
    b.total_nanos
        .cmp(&a.total_nanos)
        .then_with(|| b.count.cmp(&a.count))
        .then_with(|| a.id.cmp(&b.id))
}

/// Builds the raw `METHOD host[:port]/path` pre-key for a request. The
/// non-default port comes through `port` (`None` when default for the scheme);
/// query string, fragment, and credentials are dropped by construction.
pub(crate) fn endpoint_pre_key(
    method: &str,
    host: Option<&str>,
    port: Option<u16>,
    path: &str,
) -> String {
    let host = host.unwrap_or("");
    match port {
        Some(port) => format!("{method} {host}:{port}{path}"),
        None => format!("{method} {host}{path}"),
    }
}

/// Wrap an HTTP client so every request it sends is timed and reported in the
/// `http` section, keyed by normalized endpoint.
///
/// For `reqwest::Client` this returns the matching `reqwest-middleware`
/// `ClientWithMiddleware` (spelled `hotpath::wrap::reqwest::Client` for
/// feature-independent type names). For a ureq `ConfigBuilder` it returns the
/// same builder with hotpath's middleware appended. With the `hotpath` feature
/// disabled the argument is returned unchanged.
///
/// Optional parameter: `label` (prefixes every endpoint key).
///
/// # Examples
///
/// ```rust,ignore
/// let client = hotpath::http!(reqwest::Client::new());
/// let client = hotpath::http!(reqwest::Client::new(), label = "github");
///
/// // Normal reqwest usage from here on:
/// let resp = client.get("https://api.example.com/users/1").send().await?;
///
/// // ureq: instrument the config builder before building the agent.
/// let agent: ureq::Agent = hotpath::http!(ureq::Agent::config_builder())
///     .build()
///     .new_agent();
/// let resp = agent.get("https://api.example.com/users/1").call()?;
/// ```
#[macro_export]
macro_rules! http {
    ($client:expr) => {{
        $crate::http::init_http_state();
        $crate::InstrumentHttpClient::instrument_http($client, None)
    }};
    ($client:expr, label = $label:expr) => {{
        $crate::http::init_http_state();
        $crate::InstrumentHttpClient::instrument_http($client, Some($label.to_string()))
    }};
}
