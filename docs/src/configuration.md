# Environment Variables Configuration

`hotpath` behavior can be customized via environment variables. These take precedence over programmatic configuration (`hotpath::main` macro parameters and builder API).

## Output

| Variable | Description |
|----------|-------------|
| `HOTPATH_OUTPUT_FORMAT` | Output format: `table`, `json`, `json-pretty`, or `none`. Using `none` silences output while keeping the metrics server and MCP server active. (default: `table`) |
| `HOTPATH_OUTPUT_PATH` | Filesystem path for profiling reports. If unset, reports are written to `stdout`. When set, this env var takes precedence over programmatic `output_path` config. On Unix, use `/dev/stdout` or `/dev/stderr` to redirect to the standard streams. |
| `HOTPATH_REPORT` | Report sections spec: `all`, `auto`, an exact comma-separated list (`functions-timing`, `functions-alloc`, `functions-cpu`, `channels`, `streams`, `futures`, `rw_locks`, `mutexes`, `sql`, `http`, `server`, `io`, `threads`, `debug`), or auto with exclusions like `auto,-threads` / `-threads`. (default: `auto` - function and thread sections plus every instrumented section with data) |
| `HOTPATH_REPORT_LABEL` | Free-form name for the run, written as the top-level `label` field of the JSON report. Ignored when empty. (default: unset) |
| `HOTPATH_DISABLE_HINTS` | Set to `1` or `true` to hide the hint line printed at the end of the table report. (default: unset) |
| `HOTPATH_USER_METADATA` | Comma-separated `key=value` pairs attached to the report, e.g. `commit=abc123,env=ci`. Rendered as a `user_metadata` table under the report header and as a top-level `user_metadata` object in JSON output. Values are split on the first `=`, so they may contain `=` but not `,`; keys must be unique and non-empty, otherwise the guard panics at startup. Merged on top of `HotpathGuardBuilder::user_metadata`, env values win per key. |

## Limits

| Variable | Description |
|----------|-------------|
| `HOTPATH_LIMIT` | Maximum number of items shown in every report section except debug (functions, channels, streams, futures, locks, SQL, HTTP, server routes, I/O, threads). Set to `0` for unlimited. Per-resource env vars (e.g. `HOTPATH_FUNCTIONS_LIMIT`) take precedence. (default: unset) |
| `HOTPATH_FUNCTIONS_LIMIT` | Maximum number of functions shown in the report. Set to `0` for unlimited. (default: `15`) |
| `HOTPATH_CHANNELS_LIMIT` | Maximum number of channels shown in the report. Set to `0` for unlimited. (default: `0`) |
| `HOTPATH_STREAMS_LIMIT` | Maximum number of streams shown in the report. Set to `0` for unlimited. (default: `0`) |
| `HOTPATH_FUTURES_LIMIT` | Maximum number of futures shown in the report. Set to `0` for unlimited. (default: `0`) |
| `HOTPATH_RW_LOCKS_LIMIT` | Maximum number of RwLocks shown in the report. Set to `0` for unlimited. (default: `0`) |
| `HOTPATH_MUTEXES_LIMIT` | Maximum number of mutexes shown in the report. Set to `0` for unlimited. (default: `0`) |
| `HOTPATH_SQL_LIMIT` | Maximum number of SQL queries shown in the report. Set to `0` for unlimited. (default: `0`) |
| `HOTPATH_HTTP_LIMIT` | Maximum number of outbound HTTP endpoints shown in the report. Set to `0` for unlimited. (default: `0`) |
| `HOTPATH_SERVER_LIMIT` | Maximum number of server routes shown in the report. Set to `0` for unlimited. (default: `0`) |
| `HOTPATH_IO_LIMIT` | Maximum number of I/O wrappers shown in the report. Set to `0` for unlimited. (default: `0`) |
| `HOTPATH_THREADS_LIMIT` | Maximum number of threads shown in the report. Set to `0` for unlimited. (default: `5`) |

## Functions

| Variable | Description |
|----------|-------------|
| `HOTPATH_FOCUS` | Filter profiled functions by name. Plain text does substring matching; wrap in `/pattern/` for regex (e.g. `HOTPATH_FOCUS="/^(compute\|process)/"`). Patterns use the [`regex-lite`](https://docs.rs/regex-lite) syntax: `\b`, `\w`, `\d` and `\s` are ASCII-only and Unicode classes like `\p{..}` are unsupported, so function names are expected to be ASCII; an invalid pattern panics at startup. (default: `''`) |
| `HOTPATH_EXCLUDE_WRAPPER` | Set to `true` or `1` to calculate ratios using the sum of measured functions instead of the wrapper total. (default: `false`) |
| `HOTPATH_TIME_EXCLUSIVE` | Set to `true` or `1` to report exclusive (self) time per function, excluding time spent in nested measured calls, instead of the default inclusive time. Disables function time sampling. See [Exclusive time mode](./functions.md#exclusive-time-mode). (default: `false`) |
| `HOTPATH_ALLOC_CUMULATIVE` | Set to `true` or `1` to track cumulative memory allocations per function (including nested calls) instead of the default exclusive mode. Produces invalid results for recursive functions. (default: `false`) |
| `HOTPATH_ALLOC_METRIC` | Primary metric for alloc mode: `bytes` or `count`. Controls sorting, percentages, and displayed values in reports. (default: `bytes`) |
| `HOTPATH_KEEP_INLINE` | Set to `true` or `1` to preserve user-provided `#[inline(...)]` attributes instead of rewriting measured functions to `#[inline(never)]` under `hotpath-cpu`. This env var is read during proc-macro expansion, so run `cargo clean` before rebuilding for changes to take effect. (default: `false`) |
| `HOTPATH_FUNCTIONS_NAME_DEPTH` | Number of module segments to keep when displaying function names (including the function name itself). `1` = function name only, `2` = one module + function, `0` = unlimited (full path). When using the TUI, set this env var for the TUI process too, since the console applies name shortening in its own process. (default: `2`) |

## Time Sampling

Measure durations for only a fraction of calls to reduce profiling overhead in extremely hot code paths. Rates are fractions in `[0.0, 1.0]`: `0.1` times about 1 in 10 calls, `0.0` is count-only mode (counts, states, and queue sizes stay exact, no durations at all), `1.0` or unset measures everything. Per-resource variables take precedence over the global rate, and all env vars take precedence over the [`HotpathGuardBuilder`](https://docs.rs/hotpath/latest/hotpath/struct.HotpathGuardBuilder.html) setters. See [Profiling overhead](profiling_overhead.md#reducing-overhead-time-sampling) for details.

| Variable | Description |
|----------|-------------|
| `HOTPATH_TIME_SAMPLING_RATE` | Global sampling rate applied to all resource types below. (default: unset, measure everything) |
| `HOTPATH_FUNCTIONS_TIME_SAMPLING_RATE` | Sampling rate for function timings. (default: unset) |
| `HOTPATH_MUTEXES_TIME_SAMPLING_RATE` | Sampling rate for mutex wait & acquire timings. (default: unset) |
| `HOTPATH_RW_LOCKS_TIME_SAMPLING_RATE` | Sampling rate for RwLock wait & acquire timings. (default: unset) |
| `HOTPATH_FUTURES_TIME_SAMPLING_RATE` | Sampling rate for future poll timings. (default: unset) |
| `HOTPATH_CHANNELS_TIME_SAMPLING_RATE` | Sampling rate for channel send/receive latency timings. (default: unset) |
| `HOTPATH_IO_TIME_SAMPLING_RATE` | Sampling rate for byte-level I/O operation timings. (default: unset) |

## CPU Sampling

| Variable | Description |
|----------|-------------|
| `HOTPATH_CPU_INCLUSIVE` | Set to `true` or `1` to credit each CPU sample to every instrumented function on its stack (inclusive) instead of only the innermost one (exclusive). (default: `false`) |
| `HOTPATH_SAMPLY_WRAPPER_BIN` | Path to the `hotpath-samply` wrapper binary that autospawn launches under the `hotpath-cpu` feature. (default: `hotpath-samply`, resolved via `PATH`) |
| `HOTPATH_SAMPLY_BIN` | Path to the external `samply` binary used by the `hotpath-samply` worker. (default: `samply`, resolved via `PATH`) |

## Metrics Server

| Variable | Description |
|----------|-------------|
| `HOTPATH_METRICS_PORT` | Port for the HTTP metrics server (binds to `localhost` only). (default: `6770`) |
| `HOTPATH_METRICS_SERVER_OFF` | Set to `true` or `1` to disable the HTTP metrics server entirely. (default: `false`) |
| `HOTPATH_METRICS_AUTH_TOKEN` | When set, every request must send this exact token as the `Authorization` header value (no `Bearer` prefix) or it gets `401`. Any printable ASCII characters without whitespace; anything else panics at startup. The token travels in plaintext: the server still binds to `localhost` only, so this guards against other local processes and accidental exposure through tunnels, not a substitute for TLS. (default: `''`) |

## MCP Server

| Variable | Description |
|----------|-------------|
| `HOTPATH_MCP_PORT` | Port for the MCP (Model Context Protocol) server. (default: `6771`) |
| `HOTPATH_MCP_AUTH_TOKEN` | When set, clients must include this token in the `Authorization` header. (default: `''`) |

## Prometheus Exporter

Requires the `hotpath-prometheus` feature, see [Prometheus & Grafana](prometheus_grafana.md).

| Variable | Description |
|----------|-------------|
| `HOTPATH_PROMETHEUS_PORT` | Port for the Prometheus `/metrics` exporter. (default: `6772`) |
| `HOTPATH_PROMETHEUS_HOST` | Bind address for the exporter. Set to `0.0.0.0` when a Prometheus container must reach it through the Docker bridge gateway; pair it with an auth token. (default: `127.0.0.1`) |
| `HOTPATH_PROMETHEUS_AUTH_TOKEN` | When set, every scrape must send this token in the `Authorization` header, bare or `Bearer`-prefixed (matches Prometheus' `authorization` scrape config). Same character rules as `HOTPATH_METRICS_AUTH_TOKEN`. (default: `''`) |
| `HOTPATH_PROMETHEUS_FAST_BUCKETS` | Classic histogram bucket upper bounds, in seconds, for function, lock, channel, future and I/O durations. Comma-separated, positive, strictly ascending, at most `1000`; `+Inf` is always appended and invalid input panics at startup. Only affects the text (classic) exposition, native histograms are unaffected. (default: log-spaced 1-3 steps from `0.00000025` to `10`) |
| `HOTPATH_PROMETHEUS_SLOW_BUCKETS` | Same as `HOTPATH_PROMETHEUS_FAST_BUCKETS`, for SQL query, outbound HTTP request and server response durations. (default: log-spaced 1-3 steps from `0.0001` to `60`) |

## TUI

| Variable | Description |
|----------|-------------|
| `HOTPATH_TUI_REFRESH_INTERVAL_MS` | TUI dashboard refresh interval in milliseconds. (default: `1000`) |
| `HOTPATH_TUI_TAB` | Initial tab to display when launching the TUI: `1` (Functions), `2` (Data Flow), `3` (I/O), `4` (Threads), `5` (Debug), `6` (Tokio). (default: unset) |
| `HOTPATH_TUI_AUTO_EXPAND_LOGS` | Auto-open the logs panel once initial data arrives and pin selection to the given table index. Set to an integer (e.g. `0` for the first row, `2` for the third). (default: unset) |
| `HOTPATH_METRICS_HOST` | Host URL that the TUI console connects to for metrics data. (default: `http://localhost`) |
| `HOTPATH_METRICS_PORT` | Port that the TUI console connects to for metrics data. (default: `6770`) |
| `HOTPATH_METRICS_AUTH_TOKEN` | Token the TUI console sends in the `Authorization` header; must match the value the profiled app was started with. Can also be passed as `--metrics-auth-token`. (default: unset) |
| `HOTPATH_DISABLE_SAMPLY_LOAD` | Set to `true` or `1` to disable the `samply load` shortcut on the CPU subtab; the `'f'` keybinding and its hint are hidden. (default: `false`) |

## Other

| Variable | Description |
|----------|-------------|
| `HOTPATH_DRAIN_INTERVAL` | Interval in milliseconds between background worker sweeps of the per-thread event queues. Decrease for high-traffic apps to bound queue memory growth between sweeps, at the cost of more worker wakeups. (default: `50`) |
| `HOTPATH_THREADS_INTERVAL_MS` | Thread monitoring sample interval in milliseconds. (default: `250`) |
| `HOTPATH_TOKIO_RUNTIME_INTERVAL_MS` | Tokio runtime metrics sampling interval in milliseconds. (default: `1000`) |
| `HOTPATH_LOGS_LIMIT` | Maximum number of log entries to keep per channel, stream, or function. (default: `50`) |
| `HOTPATH_ENTRIES_LIMIT` | Maximum number of distinct entries tracked per runtime-keyed subsystem (server routes, outbound HTTP endpoints, SQL queries). Further new keys are aggregated into a single `<other>` bucket so unmatched 404 paths or dynamic SQL cannot grow memory without bound. (default: `1000`) |
| `HOTPATH_ROUTE_SCOPE` | Set to `0` to stop attributing SQL queries and outbound HTTP requests to the axum route handling the request (the `Route` column, see [axum profiling](axum_tracing.md#route-scoping-for-sql-and-http)). Overrides `HotpathGuardBuilder::route_scope`. Requires the `axum-0-8` feature. (default: `1`) |
| `HOTPATH_MAX_LOG_LEN` | Maximum character length for logged return values (`log = true`). Values exceeding this limit are truncated with `...`. (default: `1536`) |
| `HOTPATH_SHUTDOWN_MS` | If set a profiled program will shutdown after the specified ms timeout and print the performance report. (default: `''`). Use `before_shutdown` to specify before shutdown callback. |
| `HOTPATH_SOURCE_ROOT` | Overrides the `source_root` value in the JSON report's `meta` object: the path prefix, relative to the repository root, that maps the report's relative source paths back to repository paths. Without it the value is derived by locating the build workspace root within the enclosing git checkout; when that fails (e.g. a nested workspace launched from the repository root, or running outside the checkout) `source_root` and `meta.git` are omitted rather than guessed. Setting this variable both supplies the prefix and asserts that the current checkout is the one the binary was built from. (default: derived) |
