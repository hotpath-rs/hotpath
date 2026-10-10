use axum::Router;
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData as McpError, ServerHandler,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::channels::{get_channel_logs, get_channels_json};
use crate::debug::dbg::{get_dbg_logs, get_debug_dbg_entries_json};
use crate::debug::gauge::{get_debug_gauge_entries_json, get_debug_gauge_logs};
use crate::debug::val::{get_debug_val_entries_json, get_val_logs};
use crate::functions::{
    get_function_logs_alloc, get_function_logs_timing, get_functions_alloc_json,
    get_functions_timing_json,
};
use crate::futures::{get_future_logs_list, get_futures_json};
use crate::json::{
    JsonChannelLogsList, JsonFunctionAllocLogsList, JsonFunctionTimingLogsList, JsonFutureLogsList,
    JsonHttpLogsList, JsonProfilerStatus, JsonSqlLogsList, JsonStreamLogsList,
};
use crate::output::format_duration;
use crate::streams::{get_stream_logs, get_streams_json};
use crate::threads::get_threads_json;

// Accepts both a JSON number and its string form ("3"), so clients that
// stringify tool parameters keep working.
fn id_from_number_or_string<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct IdVisitor;

    impl serde::de::Visitor<'_> for IdVisitor {
        type Value = u32;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an integer id or its string form")
        }

        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<u32, E> {
            u32::try_from(v).map_err(E::custom)
        }

        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<u32, E> {
            u32::try_from(v).map_err(E::custom)
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<u32, E> {
            v.trim().parse().map_err(E::custom)
        }
    }

    deserializer.deserialize_any(IdVisitor)
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FunctionIdParam {
    #[schemars(description = "Function id from the functions_timing or functions_alloc data list")]
    #[serde(deserialize_with = "id_from_number_or_string")]
    function_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ChannelIdParam {
    #[schemars(description = "Channel id from the channels list")]
    #[serde(deserialize_with = "id_from_number_or_string")]
    channel_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StreamIdParam {
    #[schemars(description = "Stream id from the streams list")]
    #[serde(deserialize_with = "id_from_number_or_string")]
    stream_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FutureIdParam {
    #[schemars(description = "Future id from the futures list")]
    #[serde(deserialize_with = "id_from_number_or_string")]
    future_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SqlIdParam {
    #[schemars(description = "SQL query id from the sql list")]
    #[serde(deserialize_with = "id_from_number_or_string")]
    sql_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct HttpIdParam {
    #[schemars(description = "HTTP endpoint id from the http list")]
    #[serde(deserialize_with = "id_from_number_or_string")]
    http_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ServerIdParam {
    #[schemars(description = "Server route id from the server list")]
    #[serde(deserialize_with = "id_from_number_or_string")]
    server_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GaugeIdParam {
    #[schemars(description = "Gauge id from the gauges response")]
    #[serde(deserialize_with = "id_from_number_or_string")]
    gauge_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DebugIdParam {
    #[schemars(description = "Entry id from the dbg_entries or val_entries response")]
    #[serde(deserialize_with = "id_from_number_or_string")]
    debug_id: u32,
}

static MCP_SERVER_PORT: LazyLock<u16> = LazyLock::new(|| {
    std::env::var("HOTPATH_MCP_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(6771)
});

#[derive(Clone)]
pub(crate) struct HotPathMcpServer {
    tool_router: ToolRouter<Self>,
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
#[tool_router]
impl HotPathMcpServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = r#"Get execution timing metrics for all profiled functions.

Returns a JSON object with:
- profiling_mode, time_elapsed, total_elapsed_ns, caller_name
- description: whether durations are inclusive (default) or exclusive of nested measured calls (HOTPATH_TIME_EXCLUSIVE=true; async functions then report busy time summed over polls, without time suspended at .await)
- percentiles: configured percentile list (e.g. [95.0, 99.0])
- data: functions sorted by total time, each with:
  - id: function id, input for function_timing_logs
  - name: fully qualified function name (e.g. "my_app::db::query")
  - calls: number of invocations; sampled_calls: invocations that were timed
  - avg and one key per configured percentile (e.g. "p95"): formatted durations of the timed calls
  - total: formatted total duration; exact when sampled_calls == calls, otherwise extrapolated as avg * calls under time sampling
  - percent_total: total as a share of the profiled program's total time (the #[main] wrapper, or the sum of all function totals when the wrapper is excluded or in exclusive mode); extrapolated like total
  - avg, total, percentiles and percent_total are "-" when no call was timed (sampled_calls == 0)
  - location: source file, line and column (when known)
- total_count / included_count: entries measured vs entries returned in data (the list was truncated by the display limit when they differ)

Use this first to identify performance hotspots. Look for high percentile values indicating tail latency issues."#
    )]
    async fn functions_timing(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: functions_timing");

        match get_functions_timing_json() {
            Some(formatted) => Ok(CallToolResult::success(vec![Content::text(to_json(
                &formatted,
            )?)])),
            None => Ok(worker_not_ready()),
        }
    }

    #[tool(
        description = r#"Get memory allocation metrics per function (requires hotpath-alloc feature).

Returns a JSON object with the same shape as functions_timing, plus:
- profiling_mode: "alloc-bytes" (default) or "alloc-count" (HOTPATH_ALLOC_METRIC=count); selects the unit of every value below
- description: whether values are exclusive to each function or cumulative including nested calls
- total_allocated: grand total in the selected unit
- data: functions sorted by total allocation in the selected unit, each with:
  - id: function id, input for function_alloc_logs
  - name: fully qualified function name
  - calls / sampled_calls: invocations and measured invocations
  - avg and one key per configured percentile (e.g. "p95"): formatted per-call bytes or allocation counts
  - total: formatted cumulative bytes or allocation count across all calls
  - percent_total: share of total_allocated
  - location: source file, line and column (when known)
  - async functions report "N/A" for avg, total, percentiles, and percent_total
- total_count / included_count: entries measured vs entries returned in data (the list was truncated by the display limit when they differ)

Returns error if hotpath-alloc feature is not enabled. Cross-reference with functions_timing to find functions that are both slow and allocation-heavy."#
    )]
    async fn functions_alloc(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: functions_alloc");

        match get_functions_alloc_json() {
            Some(Some(formatted)) => Ok(CallToolResult::success(vec![Content::text(to_json(
                &formatted,
            )?)])),
            Some(None) => Ok(CallToolResult::error(vec![Content::text(
                "Memory profiling not available - enable hotpath-alloc feature",
            )])),
            None => Ok(worker_not_ready()),
        }
    }

    #[tool(
        description = r#"Get CPU sampling attribution per instrumented function (requires hotpath-cpu feature).

Returns a JSON envelope with:
- status: "idle", "capturing", "ready", or "error"
- captured_at_ms, capture_duration_ms: timing of the last capture (when available)
- error: failure message when status is "error"
- report: when ready, an object with time_elapsed, total_samples, attributed_samples, profile_path, and data: functions sorted by samples, each with id, name, samples, and percent
- current_session_id, current_session_path: the active sampling session

Use functions_cpu_snapshot to trigger an on-demand capture, then poll this tool until status is "ready". Returns error if hotpath-cpu feature is not enabled."#
    )]
    async fn functions_cpu(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: functions_cpu");

        #[cfg(feature = "hotpath-cpu")]
        {
            let envelope = crate::functions::cpu::get_cpu_envelope();
            Ok(CallToolResult::success(vec![Content::text(to_json(
                &envelope,
            )?)]))
        }

        #[cfg(not(feature = "hotpath-cpu"))]
        Ok(CallToolResult::error(vec![Content::text(
            "CPU profiling not available - enable hotpath-cpu feature",
        )]))
    }

    #[tool(
        description = r#"Trigger an on-demand CPU sampling snapshot (requires hotpath-cpu feature).

Starts a background capture of CPU samples collected since profiling began. Returns immediately with {"status":"capturing"}. Poll the functions_cpu tool until its status is "ready" to read the results.

Returns error if a snapshot is already in progress or the hotpath-cpu feature is not enabled."#
    )]
    async fn functions_cpu_snapshot(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: functions_cpu_snapshot");

        #[cfg(feature = "hotpath-cpu")]
        {
            if crate::functions::cpu::try_spawn_snapshot() {
                Ok(CallToolResult::success(vec![Content::text(
                    r#"{"status":"capturing"}"#,
                )]))
            } else {
                Ok(CallToolResult::error(vec![Content::text(
                    "Snapshot already in progress",
                )]))
            }
        }

        #[cfg(not(feature = "hotpath-cpu"))]
        Ok(CallToolResult::error(vec![Content::text(
            "CPU profiling not available - enable hotpath-cpu feature",
        )]))
    }

    #[tool(
        description = r#"Get metrics for all monitored channels (tokio, crossbeam, std, futures-channel).

Returns a JSON object with current_elapsed_ns, percentiles, and data: one entry per channel call site, each with:
- id: channel id, input for channel_logs
- label, has_custom_label, source, location: identification
- channel_type: "bounded[N]", "unbounded", "oneshot", or "pending" (not yet initialized)
- state: "active", "closed", or "notified"; omitted for aggregated entries (instances > 1)
- instances / closed_instances: channel instances aggregated into the entry and how many have closed
- sent_count / received_count: message counts
- sent_per_sec / received_per_sec: throughput over the channel's active window (omitted when not derivable)
- queue_size / max_queue_size: current and peak queued messages (when known)
- delay_avg and delay_percentiles: formatted time between send and receive (when sampled)
- type_name, type_size: message type

The response object (not each entry) also has total_count / included_count: entries measured vs entries returned in data (data was truncated by the display limit when they differ).

Use to track channel throughput and identify stalled, backed-up, or closed channels."#
    )]
    async fn channels(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: channels");

        let channels = get_channels_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &channels,
        )?)]))
    }

    #[tool(description = r#"Get metrics for all monitored async streams.

Returns a JSON object with current_elapsed_ns and data: one entry per stream call site, each with:
- id: stream id, input for stream_logs
- label, has_custom_label, source, location: identification
- state: "active" or "closed"; omitted for aggregated entries (instances > 1)
- instances / closed_instances: stream instances aggregated into the entry and how many have completed
- items_yielded: count of items produced
- type_name, type_size: item type

The response object (not each entry) also has total_count / included_count: entries measured vs entries returned in data (data was truncated by the display limit when they differ).

Use to track stream throughput and identify stalled streams."#)]
    async fn streams(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: streams");

        let streams = get_streams_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &streams,
        )?)]))
    }

    #[tool(description = r#"Get lifecycle metrics for all monitored futures.

Returns a JSON object with current_elapsed_ns and data: one entry per future, each with:
- id: future id, input for future_logs
- label, has_custom_label, source, location: identification
- call_count: number of future invocations observed
- total_polls: cumulative number of poll calls across invocations; sampled_polls: polls that were timed
- total_poll_duration_ns: cumulative poll time in nanoseconds
- total_poll_alloc_bytes / total_poll_alloc_count: allocations made while polling (null without hotpath-alloc)

The response object (not each entry) also has total_count / included_count: entries measured vs entries returned in data (data was truncated by the display limit when they differ).

High poll counts can indicate futures that wake frequently without making progress."#)]
    async fn futures(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: futures");

        let futures = get_futures_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &futures,
        )?)]))
    }

    #[tool(
        description = r#"Get wait and acquire-time metrics for all monitored RwLocks.

Returns a JSON object with current_elapsed_ns, percentiles, and data: one entry per lock, each with:
- id, label, has_custom_label, source, location, type_name: identification
- read_count / write_count: number of read and write acquisitions; read_sampled_count / write_sampled_count: acquisitions that were timed
- read_wait_avg / write_wait_avg and read_wait_percentiles / write_wait_percentiles: formatted time blocked before the lock was granted
- read_acquire_avg / write_acquire_avg and read_acquire_percentiles / write_acquire_percentiles: formatted time the lock was held, granted to released

The response object (not each entry) also has total_count / included_count: entries measured vs entries returned in data (data was truncated by the display limit when they differ).

High wait times indicate lock contention; high acquire times indicate long critical sections. Locks are instrumented via hotpath::rw_lock!(expr)."#
    )]
    async fn rw_locks(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: rw_locks");

        let rw_locks = crate::rw_locks::get_rw_locks_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &rw_locks,
        )?)]))
    }

    #[tool(
        description = r#"Get wait and acquire-time metrics for all monitored Mutexes.

Returns a JSON object with current_elapsed_ns, percentiles, and data: one entry per mutex, each with:
- id, label, has_custom_label, source, location, type_name: identification
- count: number of lock acquisitions; sampled_count: acquisitions that were timed
- wait_avg and wait_percentiles: formatted time blocked before the lock was granted
- acquire_avg and acquire_percentiles: formatted time the lock was held, granted to released

The response object (not each entry) also has total_count / included_count: entries measured vs entries returned in data (data was truncated by the display limit when they differ).

High wait times indicate lock contention; high acquire times indicate long critical sections. Mutexes are instrumented via hotpath::mutex!(expr)."#
    )]
    async fn mutexes(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: mutexes");

        let mutexes = crate::mutexes::get_mutexes_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &mutexes,
        )?)]))
    }

    #[tool(
        description = r#"Get byte-level I/O metrics for all instrumented Read/Write/AsyncRead/AsyncWrite values.

Returns a JSON object with current_elapsed_ns, percentiles, and data: one entry per wrapper call site, each with:
- id, label, has_custom_label, source, location, type_name: identification
- instances: wrapper instances aggregated into the entry
- read, write, flush, shutdown: per-operation stats, each with:
  - count / sampled_count: completed operations and operations that were timed
  - bytes / sampled_bytes: total bytes processed and bytes of timed operations
  - errors: failed operations (retryable WouldBlock/Interrupted conditions are not counted)
  - avg, percentiles: formatted durations; total_ns: raw total duration in nanoseconds
  - bytes_per_sec: formatted transfer rate over timed operations, in bytes per second without a /s suffix (null when nothing was timed)

The response object (not each entry) also has total_count / included_count: entries measured vs entries returned in data (data was truncated by the display limit when they differ).

Async durations span first poll to Ready, so they include async waiting time. I/O values are instrumented via hotpath::io!(expr). Wrapping the underlying resource (file, socket) measures actual resource I/O; wrapping a BufReader/BufWriter measures application-facing buffered operations."#
    )]
    async fn io(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: io");

        let io = crate::io::get_io_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(&io)?)]))
    }

    #[tool(description = r#"Get execution-time metrics for captured SQL queries.

Returns a JSON object with current_elapsed_ns, total_ns, total_calls, percentiles, and data: one entry per normalized query (parameter-varied executions merge into one bucket), sorted by total time, each with:
- id: query id, input for sql_logs
- query: normalized SQL statement text
- source: innermost instrumented function that ran the query (absent outside measured scopes)
- route: axum route template (e.g. GET /users/{id}) whose handler ran the query; absent outside AxumLayer or when route scoping is disabled. The same query under two routes yields two entries
- count: number of executions
- avg, total, and percentiles: formatted durations
- percent_total: share of total SQL time
- location: source location of the source function (when known)

The response object (not each entry) also has total_count / included_count: entries measured vs entries returned in data (data was truncated by the display limit when they differ).

Requires a SQL front-end in the profiled application: hotpath::sqlx_tracing_layer() or hotpath::toasty_tracing_layer() added to its tracing subscriber, or hotpath::instrument_diesel_sql() called before opening Diesel connections. Use sql_logs with a query id to get recent individual executions."#)]
    async fn sql(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: sql");

        let sql = crate::sql::get_sql_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(&sql)?)]))
    }

    #[tool(description = r#"Get detailed execution logs for a specific SQL query.

Returns a JSON object with id and logs: recent executions, each with index, timestamp, ago, duration (formatted), and query. Use sql first to get query IDs, then use this tool to get detailed logs."#)]
    async fn sql_logs(&self, params: Parameters<SqlIdParam>) -> Result<CallToolResult, McpError> {
        let sql_id = params.0.sql_id;
        log_debug(&format!("Tool called: sql_logs({})", sql_id));

        match crate::sql::get_sql_logs(sql_id) {
            Some(logs) => {
                let current_elapsed_ns = get_current_elapsed_ns();
                let formatted = JsonSqlLogsList::from_logs(&logs, current_elapsed_ns);
                Ok(CallToolResult::success(vec![Content::text(to_json(
                    &formatted,
                )?)]))
            }
            None => Ok(CallToolResult::error(vec![Content::text(
                "SQL query not found",
            )])),
        }
    }

    #[tool(
        description = r#"Get execution-time metrics for HTTP requests sent through hotpath's reqwest/ureq instrumentation.

Returns a JSON object with current_elapsed_ns, total_ns, total_calls, percentiles, and data: one entry per normalized endpoint (parameter-varied requests to the same route merge into one bucket, e.g. GET api.example.com/users/{id}), sorted by total time, each with:
- id: endpoint id, input for http_logs
- endpoint: normalized METHOD host/path key
- source: innermost instrumented function that sent the request (absent outside measured scopes)
- route: axum route template (e.g. GET /users/{id}) whose handler sent the request; absent outside AxumLayer or when route scoping is disabled. The same endpoint under two routes yields two entries
- count: number of requests
- errors: transport errors plus responses with status >= 400
- avg, total, and percentiles: formatted durations
- percent_total: share of total outbound HTTP time
- location: source location of the source function (when known)

The response object (not each entry) also has total_count / included_count: entries measured vs entries returned in data (data was truncated by the display limit when they differ).

Requires wrapping the reqwest client or ureq config builder with hotpath::http!(...) (or attaching ReqwestHttpMiddleware / UreqHttpMiddleware directly). Use http_logs with an endpoint id to get recent individual requests."#
    )]
    async fn http(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: http");

        let http = crate::http::get_http_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &http,
        )?)]))
    }

    #[tool(
        description = r#"Get detailed request logs for a specific HTTP endpoint.

Returns a JSON object with id and logs: recent requests, each with index, timestamp, ago, duration (formatted), and status (status code, or "error" for transport failures). Use http first to get endpoint IDs, then use this tool to get detailed logs."#
    )]
    async fn http_logs(&self, params: Parameters<HttpIdParam>) -> Result<CallToolResult, McpError> {
        let http_id = params.0.http_id;
        log_debug(&format!("Tool called: http_logs({})", http_id));

        match crate::http::get_http_logs(http_id) {
            Some(logs) => {
                let current_elapsed_ns = get_current_elapsed_ns();
                let formatted = JsonHttpLogsList::from_logs(&logs, current_elapsed_ns);
                Ok(CallToolResult::success(vec![Content::text(to_json(
                    &formatted,
                )?)]))
            }
            None => Ok(CallToolResult::error(vec![Content::text(
                "HTTP endpoint not found",
            )])),
        }
    }

    #[tool(
        description = r#"Get response-time metrics for HTTP requests served by the app's axum router, captured via hotpath::AxumLayer.

Returns a JSON object with current_elapsed_ns, total_ns, total_calls, total_alloc_bytes, percentiles, and data: one entry per matched route template (e.g. GET /users/{id}; requests that matched no route are bucketed by their normalized raw path), sorted by total time, each with:
- id: route id, input for server_logs
- route: METHOD template key
- count: number of requests served
- status_4xx / status_5xx: responses with a 4xx / 5xx status
- sql_per_request / http_per_request: average SQL queries and outbound HTTP requests issued per request (absent when not measurable)
- avg, total, and percentiles: formatted durations (measured until the response head is produced; body streaming is excluded)
- percent_total: share of total server time
- alloc: with hotpath-alloc, bytes_per_request, allocs_per_request, total_bytes, and formatted avg/total/percentiles/percent_total of memory allocated per request

The response object (not each entry) also has total_count / included_count: entries measured vs entries returned in data (data was truncated by the display limit when they differ).

Requires wrapping the router with hotpath::axum!(...) (or adding hotpath::AxumLayer::new() via Router::layer). Use server_logs with a route id to get recent individual requests."#
    )]
    async fn server(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: server");

        let server = crate::server::get_server_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &server,
        )?)]))
    }

    #[tool(
        description = r#"Get detailed request logs for a specific server route.

Returns a JSON object with id and logs: recent requests, each with index, timestamp, ago, duration (formatted), and status code. Use server first to get route IDs, then use this tool to get detailed logs."#
    )]
    async fn server_logs(
        &self,
        params: Parameters<ServerIdParam>,
    ) -> Result<CallToolResult, McpError> {
        let server_id = params.0.server_id;
        log_debug(&format!("Tool called: server_logs({})", server_id));

        match crate::server::get_server_logs(server_id) {
            Some(logs) => {
                let current_elapsed_ns = get_current_elapsed_ns();
                let formatted = JsonHttpLogsList::from_logs(&logs, current_elapsed_ns);
                Ok(CallToolResult::success(vec![Content::text(to_json(
                    &formatted,
                )?)]))
            }
            None => Ok(CallToolResult::error(vec![Content::text(
                "Server route not found",
            )])),
        }
    }

    #[tool(
        description = r#"Get CPU and memory metrics for all threads of the profiled process.

Returns a JSON object with:
- current_elapsed_ns, sample_interval_ms, thread_count
- rss_bytes / rss_bytes_max: current and peak resident set size, formatted (when available)
- total_alloc_bytes, total_dealloc_bytes, alloc_dealloc_diff: formatted process-wide totals (with hotpath-alloc)
- data: threads sorted by peak CPU usage (with hotpath-alloc: by allocation traffic, the larger of allocated and deallocated bytes), each with:
  - os_tid, name (e.g. "tokio-runtime-worker"), status, status_code
  - cpu_percent, cpu_percent_max, cpu_percent_avg: formatted CPU utilization (e.g. "12.3%", 100% per core; null until sampled)
  - alloc_bytes, dealloc_bytes, mem_diff: formatted per-thread allocation totals (with hotpath-alloc)
- total_count / included_count: entries measured vs entries returned in data (the list was truncated by the display limit when they differ)

Sampled at configurable interval (HOTPATH_THREADS_INTERVAL_MS env var, default 250ms). Useful for identifying CPU-bound threads and memory growth."#
    )]
    async fn threads(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: threads");

        let threads = get_threads_json(crate::output::Precision::Display);
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &threads,
        )?)]))
    }

    #[tool(description = r#"Get detailed timing logs for a specific function.

Returns a JSON object with function_name, location, total_invocations, and logs: recent invocations, each with invocation, duration (formatted), timestamp, ago, thread_id, and result (when the function was measured with log = true). Use functions_timing first to get function IDs, then use this tool to get detailed logs."#)]
    async fn function_timing_logs(
        &self,
        params: Parameters<FunctionIdParam>,
    ) -> Result<CallToolResult, McpError> {
        let function_id = params.0.function_id;
        log_debug(&format!(
            "Tool called: function_timing_logs({})",
            function_id
        ));

        match get_function_logs_timing(function_id) {
            Some(Some(logs)) => {
                let current_elapsed_ns = get_current_elapsed_ns();
                let formatted = JsonFunctionTimingLogsList::from_logs(&logs, current_elapsed_ns);
                Ok(CallToolResult::success(vec![Content::text(to_json(
                    &formatted,
                )?)]))
            }
            Some(None) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Function with id {} not found",
                function_id
            ))])),
            None => Ok(worker_not_ready()),
        }
    }

    #[tool(
        description = r#"Get detailed allocation logs for a specific function (requires hotpath-alloc feature).

Returns a JSON object with function_name, location, total_invocations, and logs: recent invocations, each with invocation, bytes (formatted), alloc_count, timestamp, ago, thread_id, and result. Use functions_alloc first to get function IDs, then use this tool to get detailed logs."#
    )]
    async fn function_alloc_logs(
        &self,
        params: Parameters<FunctionIdParam>,
    ) -> Result<CallToolResult, McpError> {
        let function_id = params.0.function_id;
        log_debug(&format!(
            "Tool called: function_alloc_logs({})",
            function_id
        ));

        match get_function_logs_alloc(function_id) {
            Some(Some(logs)) => {
                let current_elapsed_ns = get_current_elapsed_ns();
                let formatted = JsonFunctionAllocLogsList::from_logs(&logs, current_elapsed_ns);
                Ok(CallToolResult::success(vec![Content::text(to_json(
                    &formatted,
                )?)]))
            }
            Some(None) => Ok(CallToolResult::error(vec![Content::text(
                "Memory profiling not available - enable hotpath-alloc feature",
            )])),
            None => Ok(worker_not_ready()),
        }
    }

    #[tool(description = r#"Get detailed message logs for a specific channel.

Returns a JSON object with id, sent_logs (index, timestamp, ago, delay, message, thread_id), and received_logs (index, timestamp, ago, message, thread_id). Use channels first to get channel IDs, then use this tool to get detailed logs."#)]
    async fn channel_logs(
        &self,
        params: Parameters<ChannelIdParam>,
    ) -> Result<CallToolResult, McpError> {
        let channel_id = params.0.channel_id;
        log_debug(&format!("Tool called: channel_logs({})", channel_id));

        match get_channel_logs(channel_id) {
            Some(logs) => {
                let current_elapsed_ns = get_current_elapsed_ns();
                let formatted = JsonChannelLogsList::from_logs(&logs, current_elapsed_ns);
                Ok(CallToolResult::success(vec![Content::text(to_json(
                    &formatted,
                )?)]))
            }
            None => Ok(CallToolResult::error(vec![Content::text(
                "Channel not found",
            )])),
        }
    }

    #[tool(description = r#"Get detailed item logs for a specific stream.

Returns a JSON object with id and logs: recent yield events, each with index, timestamp, ago, message, and thread_id. Use streams first to get stream IDs, then use this tool to get detailed logs."#)]
    async fn stream_logs(
        &self,
        params: Parameters<StreamIdParam>,
    ) -> Result<CallToolResult, McpError> {
        let stream_id = params.0.stream_id;
        log_debug(&format!("Tool called: stream_logs({})", stream_id));

        match get_stream_logs(stream_id) {
            Some(logs) => {
                let current_elapsed_ns = get_current_elapsed_ns();
                let formatted = JsonStreamLogsList::from_logs(&logs, current_elapsed_ns);
                Ok(CallToolResult::success(vec![Content::text(to_json(
                    &formatted,
                )?)]))
            }
            None => Ok(CallToolResult::error(vec![Content::text(
                "Stream not found",
            )])),
        }
    }

    #[tool(description = r#"Get detailed call/poll logs for a specific future.

Returns a JSON object with id, call_count, total_polls, total_poll_duration_ns, total_poll_alloc_bytes, total_poll_alloc_count, and calls: recent invocations, each with id, state, poll_count, sampled_polls, total/max/last poll duration in nanoseconds, poll allocation stats (null without hotpath-alloc), and result. Use futures first to get future IDs, then use this tool to get detailed logs."#)]
    async fn future_logs(
        &self,
        params: Parameters<FutureIdParam>,
    ) -> Result<CallToolResult, McpError> {
        let future_id = params.0.future_id;
        log_debug(&format!("Tool called: future_logs({})", future_id));

        match get_future_logs_list(future_id) {
            Some(calls) => {
                let formatted = JsonFutureLogsList::from(&calls);
                Ok(CallToolResult::success(vec![Content::text(to_json(
                    &formatted,
                )?)]))
            }
            None => Ok(CallToolResult::error(vec![Content::text(
                "Future not found",
            )])),
        }
    }

    #[tool(description = r#"Get all gauge! entries.

Returns a JSON array of entries, each with:
- id: gauge id, input for gauge_logs
- entry_type: "gauge"
- expression: gauge key
- source, source_display, location: where the gauge is updated
- log_count: number of set/inc/dec operations
- last_value: current value

Use gauges to track numeric values that change over time like queue sizes, connection counts, or custom metrics."#)]
    async fn gauges(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: gauges");

        let gauges = get_debug_gauge_entries_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &gauges,
        )?)]))
    }

    #[tool(description = r#"Get detailed logs for a specific gauge.

Returns a JSON object with key, total_logs, and logs: recent value updates, each with index, timestamp, ago, value, and thread_id. Use gauges first to get gauge IDs, then use this tool to get detailed logs."#)]
    async fn gauge_logs(
        &self,
        params: Parameters<GaugeIdParam>,
    ) -> Result<CallToolResult, McpError> {
        let gauge_id = params.0.gauge_id;
        log_debug(&format!("Tool called: gauge_logs({})", gauge_id));

        match get_debug_gauge_logs(gauge_id) {
            Some(logs) => Ok(CallToolResult::success(vec![Content::text(to_json(
                &logs,
            )?)])),
            None => Ok(CallToolResult::error(vec![Content::text(
                "Gauge not found",
            )])),
        }
    }

    #[tool(description = r#"Get all dbg! debug entries.

Returns a JSON array of entries, each with id, entry_type ("dbg"), source, source_display, location, expression, log_count, and last_value. Use the returned IDs with dbg_logs to get detailed history."#)]
    async fn dbg_entries(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: dbg_entries");

        let entries = get_debug_dbg_entries_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &entries,
        )?)]))
    }

    #[tool(description = r#"Get all val! value tracking entries.

Returns a JSON array of entries, each with id, entry_type ("val"), expression (the val! key), source, source_display, location, log_count, and last_value. Use the returned IDs with val_logs to get detailed history."#)]
    async fn val_entries(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: val_entries");

        let entries = get_debug_val_entries_json();
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &entries,
        )?)]))
    }

    #[tool(description = r#"Get detailed logs for a specific dbg! debug entry.

Returns a JSON object with source, expression, total_logs, and logs: recent values, each with index, timestamp, ago, value, and thread_id. Use dbg_entries first to get entry IDs, then use this tool to get detailed logs."#)]
    async fn dbg_logs(&self, params: Parameters<DebugIdParam>) -> Result<CallToolResult, McpError> {
        let debug_id = params.0.debug_id;
        log_debug(&format!("Tool called: dbg_logs({})", debug_id));

        match get_dbg_logs(debug_id) {
            Some(logs) => Ok(CallToolResult::success(vec![Content::text(to_json(
                &logs,
            )?)])),
            None => Ok(CallToolResult::error(vec![Content::text(
                "Debug entry not found",
            )])),
        }
    }

    #[tool(description = r#"Get detailed logs for a specific val! debug entry.

Returns a JSON object with key, total_logs, and logs: recent value updates, each with index, timestamp, ago, value, thread_id, and source. Use val_entries first to get entry IDs, then use this tool to get detailed logs."#)]
    async fn val_logs(&self, params: Parameters<DebugIdParam>) -> Result<CallToolResult, McpError> {
        let debug_id = params.0.debug_id;
        log_debug(&format!("Tool called: val_logs({})", debug_id));

        match get_val_logs(debug_id) {
            Some(logs) => Ok(CallToolResult::success(vec![Content::text(to_json(
                &logs,
            )?)])),
            None => Ok(CallToolResult::error(vec![Content::text(
                "Value entry not found",
            )])),
        }
    }

    #[tool(
        description = r#"Get Tokio runtime metrics snapshot (requires tokio feature).

Returns a JSON object with num_workers, num_alive_tasks, global_queue_depth, optional blocking-pool, spawn, remote-schedule and IO driver counters, and workers: per-worker stats (index, park_count, busy_duration_ms, and when available poll_count, steal_count, steal_operations, overflow_count, local_queue_depth, mean_poll_time_us). Requires calling hotpath::tokio_runtime!() in the profiled application."#
    )]
    async fn tokio_runtime(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: tokio_runtime");

        #[cfg(feature = "tokio")]
        match crate::tokio_runtime::get_runtime_json() {
            Some(snapshot) => {
                return Ok(CallToolResult::success(vec![Content::text(to_json(
                    &snapshot,
                )?)]));
            }
            None => {
                return Ok(CallToolResult::error(vec![Content::text(
                    "Tokio runtime metrics not available - use hotpath::tokio_runtime!() to start collection",
                )]));
            }
        }

        #[cfg(not(feature = "tokio"))]
        Ok(CallToolResult::error(vec![Content::text(
            "Tokio runtime metrics not available - enable tokio feature",
        )]))
    }

    #[tool(description = r#"Get profiler status.

Returns a JSON object with:
- uptime: human-readable duration since profiler started (e.g. "1m 23s", "2h 5m 30s")
- pid: process id of the profiled application

Use to check if the profiler is running and how long it has been active."#)]
    async fn profiler_status(&self) -> Result<CallToolResult, McpError> {
        log_debug("Tool called: profiler_status");

        let status = JsonProfilerStatus {
            uptime: format_duration(get_current_elapsed_ns()),
            pid: std::process::id(),
        };
        Ok(CallToolResult::success(vec![Content::text(to_json(
            &status,
        )?)]))
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn get_current_elapsed_ns() -> u64 {
    crate::lib_on::current_elapsed_ns()
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for HotPathMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut server_info = Implementation::default();
        server_info.name = "hotpath".into();
        server_info.version = env!("CARGO_PKG_VERSION").into();

        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2024_11_05;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = server_info;
        info.instructions = Some(
            "hotpath profiler metrics MCP server. Provides tools to query profiling data.".into(),
        );
        info
    }
}

fn worker_not_ready() -> CallToolResult {
    CallToolResult::error(vec![Content::text(
        crate::metrics_server::WORKER_NOT_READY_MSG,
    )])
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn to_json<T: serde::Serialize>(value: &T) -> Result<String, McpError> {
    serde_json::to_string(value)
        .map_err(|e| McpError::internal_error(format!("Failed to serialize metrics: {}", e), None))
}

async fn auth_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let expected = crate::auth::token_from_env("HOTPATH_MCP_AUTH_TOKEN");
    let provided = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    if crate::auth::check_auth(expected.as_deref(), provided) {
        Ok(next.run(request).await)
    } else {
        Err(axum::http::StatusCode::UNAUTHORIZED)
    }
}

static MCP_SERVER_STARTED: OnceLock<()> = OnceLock::new();

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn start_mcp_server_once() {
    MCP_SERVER_STARTED.get_or_init(|| {
        let port = *MCP_SERVER_PORT;

        let auth_enabled = crate::auth::token_from_env("HOTPATH_MCP_AUTH_TOKEN").is_some();
        log_debug(&format!(
            "Starting MCP server on port {} (auth: {})",
            port,
            if auth_enabled { "enabled" } else { "disabled" }
        ));

        std::thread::Builder::new()
            .name("hp-mcp".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create MCP runtime");

                rt.block_on(async move {
                    let cancellation_token = CancellationToken::new();

                    let mut config = StreamableHttpServerConfig::default();
                    config.sse_keep_alive = Some(Duration::from_secs(15));
                    config.stateful_mode = true;
                    config.cancellation_token = cancellation_token.clone();

                    let service = StreamableHttpService::new(
                        || Ok(HotPathMcpServer::new()),
                        Arc::new(LocalSessionManager::default()),
                        config,
                    );

                    let app = Router::new()
                        .nest_service("/mcp", service)
                        .layer(axum::middleware::from_fn(auth_middleware));
                    #[cfg(feature = "hotpath-meta")]
                    let app = hotpath_meta::axum!(app);

                    let addr = format!("127.0.0.1:{}", port);
                    let listener = match tokio::net::TcpListener::bind(&addr).await {
                        Ok(l) => l,
                        Err(e) => {
                            log_debug(&format!("Failed to bind to {}: {}", addr, e));
                            return;
                        }
                    };

                    log_debug(&format!("Listening on http://{}/mcp", addr));

                    let _ = axum::serve(listener, app)
                        .with_graceful_shutdown(async move {
                            cancellation_token.cancelled().await;
                        })
                        .await;
                });
            })
            .expect("Failed to spawn MCP server thread");
    });
}

#[cfg(feature = "dev")]
fn log_debug(msg: &str) {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    let _ = std::fs::create_dir_all("log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("log/development.log")
    {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let sod = (secs % 86_400) as u32;
        let (hour, min, sec) = (sod / 3600, (sod % 3600) / 60, sod % 60);
        let _ = writeln!(
            file,
            "{:02}:{:02}:{:02} DEBUG [hotpath-mcp] {}",
            hour, min, sec, msg
        );
    }
}

#[cfg(not(feature = "dev"))]
fn log_debug(_msg: &str) {}

#[cfg(test)]
mod tests {
    use crate::mcp_server::*;

    #[test]
    fn id_param_accepts_number_and_string() {
        let param: SqlIdParam = serde_json::from_str(r#"{"sql_id": 3}"#).unwrap();
        assert_eq!(param.sql_id, 3);

        let param: SqlIdParam = serde_json::from_str(r#"{"sql_id": "3"}"#).unwrap();
        assert_eq!(param.sql_id, 3);

        assert!(serde_json::from_str::<SqlIdParam>(r#"{"sql_id": "abc"}"#).is_err());
        assert!(serde_json::from_str::<SqlIdParam>(r#"{"sql_id": -1}"#).is_err());
    }
}
