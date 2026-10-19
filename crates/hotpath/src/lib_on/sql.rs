//! SQL query instrumentation module - tracks query execution durations.
//!
//! Entries are keyed by *normalized* query text (see [`normalize`]), so
//! parameter-varied executions merge. Normalization runs on the worker thread.
//!
//! The write path is driven by front-ends ([`tracing_layer`], [`toasty`],
//! [`diesel`]), so it is dead when no front-end feature is on; the read path
//! stays compiled so the report/metrics wiring is feature-uniform.
#![cfg_attr(
    not(any(feature = "sqlx", feature = "diesel", feature = "toasty")),
    allow(dead_code)
)]

use crossbeam_channel::{bounded, Receiver as CbReceiver, RecvTimeoutError, Sender as CbSender};
use hdrhistogram::Histogram;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, LazyLock, Mutex as StdMutex, OnceLock};

use crate::lib_on::{meta_rw_lock, MetaRwLock};

use crate::batch::{EventProducer, EventQueueRegistry};
use crate::instant::Instant;
use crate::json::{SqlLogEntry, SqlLogs};
use crate::lib_on::hotpath_guard::{bounded_key, DRAIN_INTERVAL_MS, LOGS_LIMIT, OVERFLOW_ENTRY};
use crate::lib_on::START_TIME;
use crate::metrics_server::METRICS_SERVER_PORT;
use crate::output::MAX_LOG_LEN;

#[cfg(feature = "diesel")]
pub(crate) mod diesel;
pub(crate) mod normalize;
#[cfg(feature = "toasty")]
pub(crate) mod toasty;
#[cfg(feature = "sqlx")]
pub(crate) mod tracing_layer;

#[cfg(feature = "diesel")]
pub use diesel::instrument_diesel_sql;
#[cfg(feature = "toasty")]
pub use toasty::toasty_tracing_layer;
#[cfg(feature = "sqlx")]
pub use tracing_layer::sqlx_tracing_layer;

/// Opt-in: store the *raw* statement text in execution logs instead of the
/// normalized form. Exposes inline literals (query params rendered into the
/// statement) through the logs and the metrics server - off by default.
pub(crate) static SQL_RAW_LOGS: LazyLock<bool> =
    LazyLock::new(|| crate::shared::env_flag("HOTPATH_SQL_RAW_LOGS"));

static SQL_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

fn next_sql_id() -> u32 {
    SQL_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Events sent to the background SQL statistics collection thread.
#[derive(Debug)]
pub(crate) enum SqlEvent {
    /// Emitted when an executed query (or query stream) completes. `sql` is the
    /// raw statement text; the worker normalizes it to derive the bucket key.
    /// `timestamp_ns` is the completion time in ns since profiler start.
    /// `source` is the innermost instrumented function on the emitting thread's
    /// caller stack, `None` when the query completed outside any measured scope.
    /// `route` is the axum route template handling the request that issued the
    /// query, `None` outside the server middleware or with route scoping off.
    Executed {
        sql: Arc<str>,
        duration_nanos: u64,
        timestamp_ns: u64,
        source: Option<&'static str>,
        route: Option<&'static str>,
    },
}

/// `(route, source, normalized query)`.
pub(crate) type SqlKey = (Option<&'static str>, Option<&'static str>, String);

/// Aggregated statistics for a single normalized query executed from a single
/// source function under a single axum route. The same statement called from
/// two instrumented functions (or under two routes) produces two entries.
#[derive(Debug)]
pub(crate) struct SqlEntry {
    pub(crate) id: u32,
    pub(crate) query: String,
    pub(crate) source: Option<&'static str>,
    pub(crate) route: Option<&'static str>,
    pub(crate) count: u64,
    pub(crate) total_nanos: u64,
    hist: Option<Histogram<u64>>,
}

impl SqlEntry {
    const LOW_NS: u64 = 1;
    const HIGH_NS: u64 = crate::lib_on::MAX_DURATION_NS;
    const SIGFIGS: u8 = 3;

    fn new(
        id: u32,
        query: String,
        source: Option<&'static str>,
        route: Option<&'static str>,
    ) -> Self {
        Self {
            id,
            query,
            source,
            route,
            count: 0,
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

pub(crate) struct SqlInternalState {
    pub(crate) stats: HashMap<SqlKey, SqlEntry>,
    /// Recent executions per entry id, capped at `LOGS_LIMIT`. Log entries keep
    /// the *normalized* statement text - raw text would leak inline literals
    /// (query params), which must never reach the report or metrics server
    /// unless the user opts in via `HOTPATH_SQL_RAW_LOGS`.
    pub(crate) logs: HashMap<u32, VecDeque<SqlLogEntry>>,
}

pub(crate) struct SqlState {
    pub(crate) inner: Arc<MetaRwLock<SqlInternalState>>,
    pub(crate) shutdown_tx: StdMutex<Option<CbSender<()>>>,
    pub(crate) completion_rx: StdMutex<Option<CbReceiver<()>>>,
}

pub(crate) static SQL_STATE: OnceLock<SqlState> = OnceLock::new();

/// Runs `f` on the entries sorted for display, borrowed under the read lock:
/// nothing is cloned, so the per-entry histograms stay in the map.
pub(crate) fn with_sorted_sql_entries<R>(f: impl FnOnce(&[&SqlEntry]) -> R) -> R {
    let Some(state) = SQL_STATE.get() else {
        return f(&[]);
    };
    let guard = state.inner.read().unwrap();
    let mut stats: Vec<&SqlEntry> = guard.stats.values().collect();
    stats.sort_by(|a, b| compare_sql_entries(a, b));
    f(&stats)
}

/// Returns recent executions of the entry with the given id, newest first.
pub(crate) fn get_sql_logs(id: u32) -> Option<SqlLogs> {
    let state = SQL_STATE.get()?;
    let guard = state.inner.read().unwrap();
    let logs = guard.logs.get(&id)?;
    Some(SqlLogs {
        id,
        logs: logs.iter().rev().cloned().collect(),
    })
}

pub(crate) fn get_sql_json() -> crate::json::JsonSqlList {
    let elapsed = std::time::Duration::from_nanos(crate::lib_on::current_elapsed_ns());
    let percentiles = crate::lib_on::hotpath_guard::configured_percentiles();
    with_sorted_sql_entries(|entries| {
        crate::lib_on::report::collect_sql_json(entries, 0, elapsed, &percentiles, false)
    })
}

static EVENT_QUEUES: EventQueueRegistry<SqlEvent> = EventQueueRegistry::new();

thread_local! {
    static EVENT_PRODUCER: EventProducer<SqlEvent> = EVENT_QUEUES.register();
}

#[inline]
pub(crate) fn send_sql_event(event: SqlEvent) {
    if !EVENT_QUEUES.is_active() {
        return;
    }
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let _ = EVENT_PRODUCER.try_with(|producer| producer.push(event));
}

/// Stops producers ahead of the worker's final sweep at shutdown.
pub(crate) fn stop_sql_events() {
    EVENT_QUEUES.set_active(false);
}

fn process_sql_event(state: &mut SqlInternalState, event: SqlEvent) {
    let SqlEvent::Executed {
        sql,
        duration_nanos,
        timestamp_ns,
        source,
        route,
    } = event;

    let normalized = normalize::normalize(&sql);
    let key = bounded_key(&state.stats, (route, source, normalized), || {
        (None, None, OVERFLOW_ENTRY.to_string())
    });
    let entry = state
        .stats
        .entry(key)
        .or_insert_with_key(|(route, source, query)| {
            SqlEntry::new(next_sql_id(), query.clone(), *source, *route)
        });
    entry.count += 1;
    entry.total_nanos += duration_nanos;
    entry.record(duration_nanos);

    let logs = state.logs.entry(entry.id).or_default();
    if logs.len() >= *LOGS_LIMIT {
        logs.pop_front();
    }
    let log_text: &str = if *SQL_RAW_LOGS { &sql } else { &entry.query };
    logs.push_back(SqlLogEntry {
        index: entry.count,
        timestamp: timestamp_ns,
        duration_nanos,
        query: truncate_query(log_text),
    });
}

/// Caps the statement text stored per log entry at `MAX_LOG_LEN` chars,
/// appending `...` when cut (same convention as `log = true` return values).
fn truncate_query(sql: &str) -> String {
    let limit = *MAX_LOG_LEN;
    if sql.len() <= limit {
        return sql.to_string();
    }
    let mut end = limit.saturating_sub(3);
    while !sql.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &sql[..end])
}

fn flush_sql_buffer(buffer: &mut Vec<SqlEvent>, inner: &Arc<MetaRwLock<SqlInternalState>>) {
    if buffer.is_empty() {
        return;
    }
    if let Ok(mut shared) = inner.write() {
        for e in buffer.drain(..) {
            process_sql_event(&mut shared, e);
        }
    }
}

/// Initialize the SQL statistics collection system (called on first emitted event).
pub(crate) fn init_sql_state() -> &'static SqlState {
    SQL_STATE.get_or_init(|| {
        START_TIME.get_or_init(Instant::now);

        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let (completion_tx, completion_rx) = bounded::<()>(1);

        let inner = Arc::new(meta_rw_lock!(
            "sql_state",
            SqlInternalState {
                stats: HashMap::new(),
                logs: HashMap::new(),
            },
        ));
        let inner_clone = Arc::clone(&inner);

        EVENT_QUEUES.set_active(true);

        crate::lib_on::spawn_profiler_thread("hp-sql", move || {
            let flush_interval = std::time::Duration::from_millis(*DRAIN_INTERVAL_MS);
            let mut swept: Vec<SqlEvent> = Vec::new();

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
                    flush_sql_buffer(&mut swept, &inner_clone);
                    break;
                }

                EVENT_QUEUES.sweep(&mut swept);
                flush_sql_buffer(&mut swept, &inner_clone);
            }

            let _ = completion_tx.send(());
        })
        .expect("Failed to spawn sql-stats-collector thread");

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        SqlState {
            inner,
            shutdown_tx: StdMutex::new(Some(shutdown_tx)),
            completion_rx: StdMutex::new(Some(completion_rx)),
        }
    })
}

/// Sort entries by total time spent (slowest aggregate first), tiebreak by count.
pub(crate) fn compare_sql_entries(a: &SqlEntry, b: &SqlEntry) -> std::cmp::Ordering {
    b.total_nanos
        .cmp(&a.total_nanos)
        .then_with(|| b.count.cmp(&a.count))
        .then_with(|| a.id.cmp(&b.id))
}
