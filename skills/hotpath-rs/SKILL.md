---
name: hotpath-rs
description: Set up, use and troubleshoot hotpath, the feature-gated Rust profiler, by fetching the matching hotpath.rs docs page or agent procedure. Use when adding hotpath to a project, instrumenting Rust code with it, reading a hotpath report or hotpath.rs pull request comment, fixing a regression hotpath flagged, measuring Callgrind instruction counts, configuring hotpath-cloud CI, policies or budgets, or any question about hotpath. Not for general Rust performance work in projects that do not use or want hotpath.
allowed-tools: Bash, Read, Edit, Write, Glob, Grep
---

# Using hotpath

[hotpath](https://hotpath.rs) is a feature-gated Rust profiler: every macro is a no-op and no dependency is compiled unless the `hotpath` cargo feature is enabled. This skill does not hold the API; it routes each task to the docs page or agent procedure that does. Fetch those before writing code or answering, and follow them instead of memory.

## Fetching the docs

Every page of https://hotpath.rs is served as markdown with an `Accept: text/markdown` header, which costs far fewer tokens than HTML:

```bash
curl -fsSL -H 'Accept: text/markdown' https://hotpath.rs/functions
```

- Pages under `/agent/` are step-by-step procedures, markdown only. When a task routes to one, fetch it and follow it.
- If a fetch fails, stop and tell the user which URL failed; do not guess the API.
- A `#fragment` is not sent to the server: fetch the whole page and find the heading.
- Fetch only the pages the task needs, each once. Follow a link from a page only when you need the detail it points to.

## Routing

Paths are relative to https://hotpath.rs. Fetch the pages listed for the task, plus the page of any other subsystem it touches.

**Setup and profiling**

| Task | Pages |
|---|---|
| Add hotpath to a project | `/agent/setup` |
| Instrument functions, modules, `impl` blocks or code blocks; memory and allocations; custom global allocator | `/functions` |
| Profile async functions | `/functions`, `/data_flow` |
| Use a guard instead of `#[hotpath::main]` (libraries, tests, benchmarks); report format, output, shutdown; live TUI | `/profiling_modes`, `/configuration` |
| CPU time inside functions (samply) | `/cpu_profiling` |
| Callgrind instruction counts, alone or merged with native timing; missing or unstable counts | `/callgrind` (+ `/ci_integration` "Troubleshooting" in CI) |
| Threads | `/threads` |
| Channels, streams, futures | `/data_flow` |
| Lock contention | `/locks` |
| SQL queries, N+1 | `/sql_tracing` (+ `/axum_tracing` for per-route counts) |
| Outbound HTTP (reqwest, ureq) | `/http_tracing` |
| axum server, per-route time or memory | `/axum_tracing` |
| File, socket, compression or other byte-level I/O | `/io_tracing` |
| Tokio runtime metrics | `/tokio_runtime` |
| Debug values, custom gauges | `/debug` |
| Overhead, sampling, running in production | `/profiling_overhead`, `/faq`, `/configuration` |
| Environment variables | `/configuration` |

**hotpath-cloud**

| Task | Pages |
|---|---|
| What hotpath-cloud is, how it works, pricing | `/cloud`, `/cloud_faq` |
| Set up hotpath-cloud CI for pull requests, including CPU regression checks with instruction counts | `/agent/init_cloud` |
| Change an existing hotpath-cloud benchmark's profiling mode | `/ci_integration` ("Profiling modes"), `/regression_policy` |
| Edit or validate `hotpath/policy.toml` | `/regression_policy` |
| Absolute performance budgets | `/performance_budgets`, `/regression_policy` |
| Read a pull request comment, wait for a benchmark, fix a regression or broken budget | `/agent/cloud_cli` (+ the page of the regressed subsystem) |
| `hotpath cloud` CLI reference | `/agents_cli` |
| Data access, retention, outages, terms | `/cloud_faq`, `/terms` |

**Observability and general**

| Task | Pages |
|---|---|
| Prometheus metrics | `/prometheus_grafana` |
| Grafana dashboard | `/agent/init_grafana` |
| Query live metrics over MCP | `/mcp` |
| What hotpath can do, troubleshooting, FAQ | `/`, `/faq` |

## Rules

- hotpath version: the docs describe the latest release (`cargo search hotpath --limit 1`); add it for a new dependency. Keep an existing version (see `Cargo.lock`) when it supports the task; a hotpath cargo feature that is only not enabled yet needs enabling, not an upgrade. When the installed version lacks functionality the task needs (compare with https://github.com/hotpath-rs/hotpath/blob/main/CHANGELOG.md), upgrade to the latest release without asking: raise the requirement keeping its features, `cargo update -p hotpath`, no other dependency changes, build with and without the project's hotpath features, and tell the user the old and new version, why and what code changed. If it cannot be made to build, restore the previous `Cargo.toml`/`Cargo.lock` state, and report the failure and what stays unavailable.
- Never add `hotpath` or any `hotpath-*` feature to `default` features; profiling stays opt-in.
- Verify instrumentation compiles both ways: `cargo check` and `cargo check --features hotpath`.
- Wrapper macros return `hotpath::wrap::` types (no-op with the feature off). Do not silently rewrite public APIs to fit them; tell the user.
- When the docs and your memory disagree, the docs win. Cite the page you relied on.
- Do not commit or push unless the user asks.
