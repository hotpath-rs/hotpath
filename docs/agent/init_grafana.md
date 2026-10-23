# Build a hotpath Grafana dashboard

Instructions for an AI coding agent. Build a [Grafana](https://grafana.com) dashboard on top of the metrics that [hotpath](https://hotpath.rs) exposes through its Prometheus endpoint. The dashboard is organized in performance layers: SQL queries and outbound HTTP calls attributed to the axum route that issued them, memory and time per request, then channels, locks, the Tokio runtime and threads. Each panel answers a concrete question instead of plotting raw series.

## Scope

This setup only builds the dashboard JSON file. It does not configure Prometheus, Grafana or the app's metrics endpoint: no scrape jobs, data sources, auth tokens, Grafana API imports or Prometheus restarts. Those setups differ between projects (local, docker-compose, Kubernetes, Grafana Cloud, ...) and are owned by whoever runs the infrastructure, so it assumes they are already in place:

- the app runs with the `hotpath-prometheus` feature and exposes `/metrics`;
- Prometheus scrapes it with native histograms enabled;
- Grafana has that Prometheus as a data source.

If any of these is missing, point the user to https://hotpath.rs/prometheus_grafana and continue with the dashboard; do not edit Prometheus or Grafana config, and do not change `Cargo.toml` features or deployment env vars.

## Documentation

The hotpath docs are the reference for every metric, label and environment variable; this page holds the procedure and the dashboard queries, which the docs do not have. Every page of https://hotpath.rs is served as markdown when requested with an `Accept: text/markdown` header:

```bash
curl -sL -H 'Accept: text/markdown' https://hotpath.rs/prometheus_grafana
```

Fetch https://hotpath.rs/prometheus_grafana before building the dashboard, and check every metric and label name a panel uses against its "Available metrics" tables instead of relying on memory. Keep `-L` (some pages redirect) and the header. A `#fragment` is not sent to the server: fetch the whole page and find the heading.

The site documents the latest hotpath release, which can be newer than the version the project uses. Never change the project's `hotpath` version because the docs show a newer one. If the docs describe something that version does not have, tell the user it needs a newer hotpath instead of working around it.

| Page | Covers |
|---|---|
| https://hotpath.rs/prometheus_grafana | metrics endpoint, scrape config, native and classic histogram queries, time sampling, every metric family with its labels, exporter environment variables |
| https://hotpath.rs/axum_tracing | route scoping, behind every per-route panel |
| https://hotpath.rs/profiling_overhead | time sampling, behind the profiling overhead panel |
| https://hotpath.rs/tokio_runtime | Tokio metrics and which need `tokio_unstable` |

## Steps

### 1. Inspect the project

- Check whether hotpath is already set up: a `hotpath` dependency, `#[hotpath::main]` (or `HotpathGuardBuilder`) and instrumented code. If it is not, stop and tell the user to set up profiling first (https://hotpath.rs/agent/init_lib); do not instrument the project as part of building the dashboard.
- Record which subsystems the project instruments. They decide which dashboard rows to build:

| Instrumentation in the code | Dashboard row |
|---|---|
| `hotpath::axum!(router)` or `.layer(hotpath::AxumLayer::new())` | Routes, and every per-route panel |
| sqlx / diesel / toasty SQL layer | SQL |
| `hotpath::http!(client)` | Outbound HTTP |
| `hotpath-alloc` feature | Memory |
| `#[hotpath::measure]` / `#[hotpath::measure_all]` / `hotpath::measure_block!` | Functions, profiling overhead |
| `hotpath::channel!` | Channels |
| `hotpath::rw_lock!` / `hotpath::mutex!` | Locks |
| `hotpath::tokio_runtime!()` | Tokio runtime |
| `threads` feature (on by default) | Threads |

- Skip any row whose instrumentation is missing; an empty panel is noise. Mention the skipped rows to the user with the macro that would enable them, but do not add instrumentation unless they ask.

### 2. Check the emitted metrics (optional)

If the app's metrics endpoint is already reachable (default `http://127.0.0.1:6772/metrics`), list the metric families it emits to confirm step 1. Families with no data are omitted from the scrape:

```bash
curl -s http://127.0.0.1:6772/metrics | grep -v '^#' | cut -d'{' -f1 | sort -u
```

Do not start the app or change its configuration for this; when the endpoint is not reachable, rely on the code inspection alone.

Every query below assumes **native histograms**: `histogram_sum()`, `histogram_count()` and `histogram_quantile()` on the bare metric name. If the user says their Prometheus stores classic histograms, rewrite every histogram query to the classic form, as "Classic histogram queries" in https://hotpath.rs/prometheus_grafana shows.

### 3. Build the dashboard

Write the dashboard as Grafana JSON to `grafana/hotpath-dashboard.json` (or next to the project's existing dashboards). Requirements:

- A `datasource` template variable of type `prometheus`, used by every panel and every query variable, so the file imports into any Grafana.
- A `route` query variable, `label_values(hotpath_server_requests_total, route)`, multi-value with an "All" option (`.*`), only when the routes row exists.
- Textbox variables `overhead_ns` (default `50`), `threshold` (default `1`) and `min_share` (default `1`) for the profiling overhead panel.
- One collapsible row per layer from step 1, in the order below.
- Ranking panels ("top 10 ...") use **instant** queries over `$__range`, rendered as horizontal bar gauges, with the legend set to the grouping label (e.g. `{{query}}`, `{{route}}`). Set the unit per panel: `percent`, `s`, `bytes`, `short`.
- Long label values (SQL text, function paths) need a wide panel and value truncation off, or a table panel.

#### SQL

Share of total SQL time per query:

```promql
sort_desc(topk(10,
  100 * sum by (query) (histogram_sum(increase(hotpath_sql_duration_seconds[$__range])))
  / on() group_left()
  sum(histogram_sum(increase(hotpath_sql_duration_seconds[$__range])))
))
```

Slow queries, p95 above 500ms:

```promql
sort_desc(histogram_quantile(0.95, sum by (query) (increase(hotpath_sql_duration_seconds[$__range]))) > 0.500)
```

#### Routes (axum)

Share of total response time per route:

```promql
sort_desc(topk(10,
  100 * sum by (route) (histogram_sum(increase(hotpath_server_duration_seconds[$__range])))
  / on() group_left()
  sum(histogram_sum(increase(hotpath_server_duration_seconds[$__range])))
))
```

Slowest routes by p95:

```promql
sort_desc(topk(10, histogram_quantile(0.95, sum by (route) (increase(hotpath_server_duration_seconds[$__range])))))
```

SQL time per request, by query, for the selected `$route` (only with the SQL row):

```promql
sort_desc(topk(10,
  sum by (query) (histogram_sum(increase(hotpath_sql_duration_seconds{route=~"$route"}[$__range])))
  / scalar(sum(increase(hotpath_server_scoped_requests_total{route=~"$route"}[$__range])))
))
```

SQL queries per request, the N+1 detector (only with the SQL row):

```promql
sort_desc(topk(10,
  sum by (route) (increase(hotpath_server_sql_calls_total[$__range]))
  / (sum by (route) (increase(hotpath_server_scoped_requests_total[$__range])) > 0)
))
```

#### Outbound HTTP

HTTP calls per request (needs the routes row):

```promql
sort_desc(topk(10,
  (sum by (route) (increase(hotpath_server_http_calls_total[$__range])) > 0)
  / (sum by (route) (increase(hotpath_server_scoped_requests_total[$__range])) > 0)
))
```

HTTP time per request; `route!=""` drops calls made outside a request, e.g. background jobs:

```promql
sort_desc(topk(10,
  sum by (route) (histogram_sum(increase(hotpath_http_duration_seconds{route!=""}[$__range])))
  / (sum by (route) (increase(hotpath_server_scoped_requests_total[$__range])) > 0)
))
```

Without axum, rank endpoints by total time instead: `sort_desc(topk(10, sum by (endpoint) (histogram_sum(increase(hotpath_http_duration_seconds[$__range])))))`.

#### Memory (`hotpath-alloc`)

Bytes allocated per request, by route:

```promql
sort_desc(topk(10,
  sum by (route) (increase(hotpath_server_alloc_bytes_total[$__range]))
  / (sum by (route) (increase(hotpath_server_scoped_requests_total[$__range])) > 0)
))
```

Bytes allocated per request, by function, for the selected `$route`. Allocations are exclusive by default: each function reports only what it allocates itself.

```promql
sort_desc(topk(10,
  sum by (function) (increase(hotpath_function_route_alloc_bytes_total{route=~"$route"}[$__range]))
  / scalar(sum(increase(hotpath_server_scoped_requests_total{route=~"$route"}[$__range])))
))
```

Without axum, use `sort_desc(topk(10, sum by (function) (increase(hotpath_function_alloc_bytes_total[$__range]))))`.

#### Functions

Time per request, by function, for the selected `$route`. Dividing by timed calls and multiplying by calls keeps it correct under time sampling. The time is inclusive of nested instrumented calls.

```promql
sort_desc(topk(10,
  sum by (function) (increase(hotpath_function_route_duration_seconds_total{route=~"$route"}[$__range]))
  / (sum by (function) (increase(hotpath_function_route_timed_calls_total{route=~"$route"}[$__range])) > 0)
  * sum by (function) (increase(hotpath_function_route_calls_total{route=~"$route"}[$__range]))
  / scalar(sum(increase(hotpath_server_scoped_requests_total{route=~"$route"}[$__range])))
))
```

Without axum, share of total instrumented time per function:

```promql
sort_desc(topk(10,
  100 * sum by (function) (histogram_sum(increase(hotpath_function_duration_seconds[$__range])))
  / on() group_left()
  sum(histogram_sum(increase(hotpath_function_duration_seconds[$__range])))
))
```

#### Channels

Every channel query groups by `source`, `label`, `iter` and `payload`, the labels that identify one channel: `label` is empty for channels created without one, so grouping by it alone would merge unrelated call sites, and a generic helper can create channels of several payload types at one call site. Use `{{label}} {{source}} {{payload}}` as the legend.

Max queue size since start; the capacity is in the `type` label. A queue at capacity means consumers are not keeping up.

```promql
sort_desc(topk(10, max by (source, label, iter, type, payload) (hotpath_channel_max_queue_size)))
```

Throughput, messages received per second:

```promql
sort_desc(topk(10, sum by (source, label, iter, type, payload) (rate(hotpath_channel_received_total[$__range]))))
```

p95 send-to-receive latency:

```promql
sort_desc(topk(10, histogram_quantile(0.95, sum by (source, label, iter, type, payload) (increase(hotpath_channel_delay_seconds[$__range])))))
```

#### Locks

p95 wait time (contention) and p95 hold time per RwLock and side, grouped by the call-site labels like the channels (legend `{{label}} {{source}} {{op}}`):

```promql
sort_desc(topk(10, histogram_quantile(0.95, sum by (source, label, iter, op) (increase(hotpath_rwlock_wait_seconds[$__range])))))
sort_desc(topk(10, histogram_quantile(0.95, sum by (source, label, iter, op) (increase(hotpath_rwlock_acquire_seconds[$__range])))))
```

For mutexes use `hotpath_mutex_wait_seconds` and `hotpath_mutex_acquire_seconds`, grouped by `(source, label, iter)`.

#### Tokio runtime

One table panel with one instant query per column, each in table format grouped by `worker`, merged with the "Merge" transformation:

```promql
sum by (worker) (increase(hotpath_tokio_worker_parks_total[$__range]))
sum by (worker) (increase(hotpath_tokio_worker_polls_total[$__range]))
sum by (worker) (increase(hotpath_tokio_worker_steals_total[$__range]))
sum by (worker) (increase(hotpath_tokio_worker_busy_seconds_total[$__range]))
100 * sum by (worker) (rate(hotpath_tokio_worker_busy_seconds_total[$__range]))
sum by (worker) (hotpath_tokio_worker_local_queue_depth)
sum by (worker) (increase(hotpath_tokio_worker_busy_seconds_total[$__range]))
  / (sum by (worker) (increase(hotpath_tokio_worker_polls_total[$__range])) > 0)
```

Columns: Parks, Polls, Steals, Busy, Busy %, Local queue, Mean poll. Some of these need `RUSTFLAGS="--cfg tokio_unstable"`; tell the user if the family is missing from the scrape.

#### Threads

Peak CPU per thread, and total bytes allocated per thread (`hotpath-alloc`):

```promql
sort_desc(topk(10, hotpath_thread_cpu_percent_max))
sort_desc(topk(10, hotpath_thread_alloc_bytes_total))
```

Retained bytes per thread (allocated minus deallocated since start), as a time series; a line that keeps rising under steady load points to a thread holding memory:

```promql
hotpath_thread_alloc_bytes_total - hotpath_thread_dealloc_bytes_total
```

Memory freed on another thread than the one that allocated it (e.g. buffers handed through a channel) makes one thread drift up and the other down, so read a rising line together with the process-level `hotpath_alloc_bytes_total - hotpath_dealloc_bytes_total`.

Legend `{{name}}`. Unnamed threads show up as `thread_N`; suggest `std::thread::Builder::name` for the important ones.

#### Profiling overhead

Functions where the per-call profiling cost (`$overhead_ns`) exceeds `$threshold` percent of the average call, limited to functions above `$min_share` percent of total instrumented time. An empty panel is the good outcome.

```promql
sort_desc(
  (
    100 * ($overhead_ns * 1e-9) / (
      sum by (function) (histogram_sum(increase(hotpath_function_duration_seconds[$__range])))
      / (sum by (function) (histogram_count(increase(hotpath_function_duration_seconds[$__range]))) > 0)
    )
    > $threshold
  )
  and on (function)
  (
    100 * sum by (function) (histogram_sum(increase(hotpath_function_duration_seconds[$__range])))
    / scalar(sum(histogram_sum(increase(hotpath_function_duration_seconds[$__range]))))
    > $min_share
  )
)
```

Functions listed here are candidates for removing `#[hotpath::measure]` or enabling time sampling (https://hotpath.rs/profiling_overhead).

### 4. Validate the dashboard

Check the file is valid JSON with `jq . grafana/hotpath-dashboard.json`. If the user gives a Prometheus URL that already scrapes the app, run each panel query through its HTTP API, read-only, with `$__range` replaced by a concrete window (e.g. `1h`) and the variables by their defaults, and fix any query that returns an error:

```bash
curl -sG http://127.0.0.1:9090/api/v1/query --data-urlencode 'query=<promql>'
```

An empty `result` is fine when the app has not served that kind of traffic yet; a `"status":"error"` is not.

### 5. Summarize

Tell the user:

- where the dashboard file is, and that they import it themselves (Grafana UI: Dashboards, New, Import) or through their own provisioning;
- which rows the dashboard contains, and which were skipped with the instrumentation that would enable them;
- how to read the first panels to look at: SQL queries per request above ~5 is an N+1 candidate, a channel at max capacity is a bottleneck, a non-empty profiling overhead panel means some instrumentation costs more than it is worth.
