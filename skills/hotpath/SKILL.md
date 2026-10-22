---
name: hotpath
description: Use hotpath, the Rust profiler, in a project that already has it or is adding it. Knows which hotpath.rs documentation page to fetch for every task - function timing and allocations, CPU sampling, threads, channels, streams, futures, locks, SQL, HTTP clients, axum servers, byte-level I/O, Tokio runtime metrics, debug helpers, reports, the live TUI, profiling overhead, environment variables, MCP, Prometheus and Grafana, and hotpath Cloud CI, regression policies and performance budgets. Use when the user wants to profile, instrument, read a hotpath report, configure hotpath, or answer any question about hotpath.
allowed-tools: Bash, Read, Edit, Write, Glob, Grep
---

# Using hotpath

[hotpath](https://hotpath.rs) is a feature-gated Rust profiler: every macro is a no-op and no dependency is compiled unless the `hotpath` cargo feature is enabled. This skill does not hold the API. It maps tasks to the documentation pages that do; fetch the matching pages before writing code or answering, and follow them instead of relying on memory.

For a first-time setup in a crate that does not depend on hotpath yet, the `hotpath_init` skill holds the procedure (or run `hotpath init`).

hotpath Cloud benchmarks every pull request in CI, compares the reports with their baseline under a regression policy and performance budgets, and posts the verdict as a pull request comment and check. Start any Cloud task with https://hotpath.rs/cloud, then follow the Cloud pages below. For reading Cloud verdicts and fixing regressions, the `hotpath_cloud` skill holds the CLI workflow.

## Fetching the docs

Every page of https://hotpath.rs is served as markdown when requested with an `Accept: text/markdown` header. Always send it: the markdown is much smaller than the HTML page and costs far fewer tokens.

```bash
curl -sL -H 'Accept: text/markdown' https://hotpath.rs/functions
```

- Keep `-L` (some pages redirect) and the header (there are no `.md` URLs).
- A `#fragment` is not sent to the server: fetch the whole page and find the heading.
- Fetch only the pages the task needs. Pages link to each other; follow a link when the page you read points to it for the detail you need.
- The site documents the latest hotpath release, which can be newer than the version the project uses (check `Cargo.lock` or `Cargo.toml`). Never change the project's `hotpath` version because the docs show a newer one. If the docs describe something that version does not have, tell the user it needs a newer hotpath instead of working around it.
- When the docs and your memory of the API disagree, the docs win.

## Pages

### Profiling

| Page | Covers |
|---|---|
| https://hotpath.rs/ | overview, feature list, AI setup with `hotpath init`, manual installation, basic setup, example report |
| https://hotpath.rs/profiling_modes | static reports, `#[hotpath::main]` vs `HotpathGuardBuilder`, report configuration, timed shutdown, live TUI dashboard, optional vs non-optional dependency |
| https://hotpath.rs/profiling_overhead | zero overhead when disabled, overhead per profiled type (functions, locks, channels, futures, streams), time sampling to reduce overhead |
| https://hotpath.rs/functions | `#[hotpath::main]`, `#[hotpath::measure]`, async functions and `future = true`, `#[hotpath::measure_all]`, `#[hotpath::skip]`, `measure_block!`, memory and allocation profiling modes, custom inner allocator, `CountingAllocator` with `HotpathGuardBuilder` |
| https://hotpath.rs/cpu_profiling | samply CPU sampling, macOS and Linux permissions, exclusive vs inclusive attribution, inlined functions, symbol attribution for impl methods |
| https://hotpath.rs/threads | per-thread CPU and memory monitoring, metrics reference, reading the output |
| https://hotpath.rs/data_flow | `channel!`, `stream!`, `future!` and `future_fn`: wrapped types, supported libraries and their cargo features, optional config, call-site aggregation and `iter = true`, `capacity` |
| https://hotpath.rs/locks | `mutex!`, `rw_lock!`: supported libraries and their cargo features, limits, wrapped types |
| https://hotpath.rs/sql_tracing | SQL normalization, source function attribution, sqlx and SeaORM tracing layer, EnvFilter caveat, Diesel instrumentation, Toasty, output limits, live SQL metrics, raw statement logs |
| https://hotpath.rs/http_tracing | reqwest client wrapping, labels, existing reqwest-middleware stacks, ureq agents, endpoint normalization, source attribution, error tracking, output limits, what is measured |
| https://hotpath.rs/axum_tracing | axum router wrapping, existing middleware stacks, route bucketing, error tracking, route scoping of SQL and HTTP, queries and memory per route, output limits |
| https://hotpath.rs/io_tracing | `io!`, measured layers, compression encoders, cancellation caveat, I/O report |
| https://hotpath.rs/tokio_runtime | `tokio_runtime!()` setup, Tokio `RuntimeMetrics`, worker threads, stable and `tokio_unstable` metrics |
| https://hotpath.rs/debug | `hotpath::dbg!`, `hotpath::val!`, `hotpath::gauge!` custom metrics |

### hotpath Cloud

| Page | Covers |
|---|---|
| https://hotpath.rs/cloud | what hotpath Cloud does, AI and manual setup, pricing |
| https://hotpath.rs/ci_integration | GitHub Actions benchmark workflow, pull requests from the same repository, failing the job, pull requests from forks and the relay workflow, CI environment variables, several benchmarks in one repository, troubleshooting |
| https://hotpath.rs/regression_policy | the `hotpath/policy.toml` file: creating, which file a run uses, validating, every section and key, pull request comment and check |
| https://hotpath.rs/performance_budgets | absolute limits per function, query, request, route, lock, channel, I/O and thread: rule keys, units, broken budgets |
| https://hotpath.rs/agents_cli | the `hotpath cloud` CLI: install and authenticate, `repos`, `benchmarks`, `report`, `diff`, `validate-policy`, `init`, output and exit codes |
| https://hotpath.rs/cloud_faq | GitHub App data access, using hotpath without Cloud, pricing for open source, report retention, CI behavior when the backend is down |

### More

| Page | Covers |
|---|---|
| https://hotpath.rs/prometheus_grafana | Prometheus metrics endpoint, scrape config, authentication, Grafana queries, time sampling, every exported metric |
| https://hotpath.rs/mcp | MCP server: setup, available tools, configuration, Claude Code setup with and without authentication, example queries |
| https://hotpath.rs/configuration | every environment variable: output, limits, functions, time and CPU sampling, metrics server, MCP server, Prometheus exporter, TUI |
| https://hotpath.rs/adoption | open source projects using hotpath |
| https://hotpath.rs/faq | overhead, metric types, async and Tokio profiling, memory profiling, profiling in production |
| https://hotpath.rs/sponsorship | sponsoring hotpath development |
| https://hotpath.rs/terms | privacy policy and terms of service of hotpath Cloud |

## Which pages to fetch

Fetch every page listed for the task; add the pages of any other subsystem the task touches.

| Task | Pages |
|---|---|
| Add hotpath to a crate, cargo features passthrough | `hotpath_init` skill, `/`, `/profiling_modes` |
| Instrument functions, methods, modules, impl blocks or code blocks | `/functions` |
| Profile async functions | `/functions`, `/data_flow` |
| Measure memory and allocations, or combine with a custom global allocator | `/functions` |
| Use hotpath without `#[hotpath::main]` (libraries, tests, benchmarks) | `/profiling_modes`, `/functions` |
| Change what the report shows, its format, output file or shutdown timing | `/profiling_modes`, `/configuration` |
| Watch a running program live (TUI) | `/profiling_modes`, `/configuration` |
| Find where CPU time goes inside functions | `/cpu_profiling` |
| Monitor threads | `/threads` |
| Wrap channels, streams or futures | `/data_flow` |
| Find lock contention | `/locks` |
| Profile SQL queries (sqlx, SeaORM, Diesel, Toasty), find N+1 queries | `/sql_tracing`, plus `/axum_tracing` for per-route counts |
| Profile outbound HTTP requests (reqwest, ureq) | `/http_tracing` |
| Profile an axum server, time or memory per route | `/axum_tracing` |
| Measure file, socket, compression or other byte-level I/O | `/io_tracing` |
| Tokio runtime and worker metrics | `/tokio_runtime` |
| Log values, debug prints or custom gauges | `/debug` |
| Reduce profiling overhead, sample calls, decide whether to keep it on in production | `/profiling_overhead`, `/faq`, `/configuration` |
| Set or look up an environment variable | `/configuration` |
| Export metrics to Prometheus, build Grafana dashboards | `/prometheus_grafana` (the `hotpath_init_grafana` skill builds a dashboard) |
| Query live metrics from an AI agent over MCP | `/mcp` |
| Benchmark pull requests in CI | `/cloud`, `/ci_integration` (the `hotpath_init_cloud` and `hotpath_init_cloud_forks` skills set it up) |
| Decide what counts as a regression, edit or validate `hotpath/policy.toml` | `/regression_policy` |
| Set absolute limits that must never be exceeded | `/performance_budgets`, `/regression_policy` |
| Read a hotpath.rs pull request comment, fix a regression or a broken budget | `/agents_cli` (the `hotpath_cloud` skill holds the workflow), `/regression_policy`, `/performance_budgets`, plus the page of the regressed subsystem |
| What hotpath Cloud is, how it works, how to start or what it costs | `/cloud`, `/cloud_faq` |
| Questions about Cloud data access, retention, pricing or outages | `/cloud_faq`, `/terms` |
| General questions about what hotpath can do | `/`, `/faq` |

## Rules

- Never add `hotpath` or any `hotpath-*` feature to the crate's `default` features; profiling must stay opt-in. Keep the existing default features as they are.
- Verify instrumentation changes compile both ways: `cargo check` and `cargo check --features hotpath`.
- Wrapper macros return `hotpath::wrap::` types; when they leak into signatures or struct fields, update them and tell the user they are plain no-op wrappers with the feature off.
- Cite the docs page you relied on when you answer a question.
