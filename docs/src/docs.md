# Getting Started

## AI Setup (Recommended)

The quickest way to set up hotpath is to let your own AI coding agent do it. Paste this prompt into Claude Code, Codex, OpenCode, Cursor or any other coding agent, inside your project repo:

```text
Set up hotpath-rs profiling in this Rust project using the instructions at https://hotpath.rs/agent/setup
```

The agent inspects your project, including workspaces and any existing hotpath setup, adds the feature-gated dependency, instruments `main` and a small starting set of hot functions, channels and locks, and verifies that everything compiles with profiling enabled and disabled. When the project has a workload that exits, it runs it and shows you the first report. It then explains how to run profiling and the live TUI. It finally asks once whether to optionally set up [hotpath-cloud](cloud_docs.md) to benchmark pull requests in CI.

The prompt is a one-time project setup. The `hotpath-rs` [agent skill](https://github.com/hotpath-rs/hotpath/blob/main/skills/hotpath-rs/SKILL.md) is what stays: reusable hotpath knowledge your agent loads in every later session. To install it, for example in a repo that already uses hotpath, run:

```bash
npx skills add hotpath-rs/hotpath --skill hotpath-rs
```

The skill maps every hotpath task, from instrumenting a crate to fixing a regression flagged in CI, to the documentation page that covers it, so your agent fetches the current docs instead of relying on its memory. Your agent remains in control: you review and approve edits through its regular permission prompts.

## Manual installation

Add to your `Cargo.toml`:

```toml
[dependencies]
hotpath = "{{HOTPATH_VERSION}}"

[features]
hotpath = ["hotpath/hotpath"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
hotpath-prometheus = ["hotpath", "hotpath/hotpath-prometheus"]
```

This config ensures that the lib has no compile time or runtime overhead unless explicitly enabled via one of the `hotpath` features. Every profiling feature also enables `hotpath`, so a single flag like `--features=hotpath-alloc` is enough. All the lib dependencies are optional (i.e. not compiled) and all macros are noop unless profiling is enabled.

## Basic setup

You'll need only `#[hotpath::main]` and `#[hotpath::measure]` macros to get started:

```rust
#[hotpath::measure]
fn sync_function(sleep: u64) {
    std::thread::sleep(Duration::from_nanos(sleep));
    let vec1 = vec![1, 2, 3];
    std::hint::black_box(&vec1); // force mem allocation
}

#[hotpath::measure]
async fn async_function(sleep: u64) {
    tokio::time::sleep(Duration::from_nanos(sleep)).await;
}

// When using with tokio, place the #[tokio::main] first
#[tokio::main]
#[hotpath::main]
async fn main() {
    for i in 0..1000 {
        sync_function(i);
        async_function(i * 2).await;

        hotpath::measure_block!("custom_block", {
            std::thread::sleep(Duration::from_nanos(i * 3))
        });
    }
}
```

Now, run your program with `hotpath` (and optionally `hotpath-alloc`) feature:

```bash
cargo run --features='hotpath-alloc'
```

On exit it will print a report with timings, memory allocations and thread usage metrics:

```
[hotpath] 1.20s | timing, alloc, threads

timing - Function execution time metrics.
+------------------------------+-------+----------+----------+----------+---------+
| Function                     | Calls | Avg      | P95      | Total    | % Total |
+------------------------------+-------+----------+----------+----------+---------+
| docs_example::main           | 1     | 1.20 s   | 1.20 s   | 1.20 s   | 100.00% |
+------------------------------+-------+----------+----------+----------+---------+
| docs_example::async_function | 1,000 | 1.16 ms  | 1.24 ms  | 1.16 s   | 96.84%  |
+------------------------------+-------+----------+----------+----------+---------+
| custom_block                 | 1,000 | 16.86 µs | 29.50 µs | 16.86 ms | 1.41%   |
+------------------------------+-------+----------+----------+----------+---------+
| docs_example::sync_function  | 1,000 | 12.16 µs | 23.84 µs | 12.17 ms | 1.02%   |
+------------------------------+-------+----------+----------+----------+---------+

alloc-bytes - Exclusive allocation bytes by each function (excluding nested calls).
Total: 100.1 KB
+------------------------------+-------+---------+---------+---------+---------+
| Function                     | Calls | Avg     | P95     | Total   | % Total |
+------------------------------+-------+---------+---------+---------+---------+
| docs_example::main           | 1     | 88.4 KB | 88.4 KB | 88.4 KB | 88.29%  |
+------------------------------+-------+---------+---------+---------+---------+
| docs_example::sync_function  | 1,000 | 12 B    | 12 B    | 11.7 KB | 11.71%  |
+------------------------------+-------+---------+---------+---------+---------+
| custom_block                 | 1,000 | 0 B     | 0 B     | 0 B     | 0.00%   |
+------------------------------+-------+---------+---------+---------+---------+
| docs_example::async_function | 1,000 | 0 B     | 0 B     | 0 B     | 0.00%   |
+------------------------------+-------+---------+---------+---------+---------+

threads - Thread CPU and memory statistics. (Max RSS: 8.6 MB, Alloc: 108.5 KB, Dealloc: 123.8 KB, Diff: -15.3 KB, 4/14)
+-----------------+------+------+----------+---------+----------+
| Thread          | Max% | Avg% | Alloc    | Dealloc | Diff     |
+-----------------+------+------+----------+---------+----------+
| main            | 4.5% | 3.8% | 100.1 KB | 46.0 KB | 54.0 KB  |
+-----------------+------+------+----------+---------+----------+
| tokio-rt-worker | 0.0% | 0.0% | 79 B     | 70.1 KB | -70.0 KB |
+-----------------+------+------+----------+---------+----------+
| tokio-rt-worker | 0.0% | 0.0% | 4.2 KB   | 471 B   | 3.8 KB   |
+-----------------+------+------+----------+---------+----------+
| tokio-rt-worker | 0.0% | 0.0% | 3.3 KB   | 407 B   | 2.9 KB   |
+-----------------+------+------+----------+---------+----------+
```

## Quick demo

Other than the SSH demo an easy way to quickly try the [ratatui.rs](https://ratatui.rs/)-powered TUI is to run it in **auto-instrumentation mode**. The TUI process profiles itself and displays its own performance metrics in real time.

First, install `hotpath` CLI with auto-instrumentation and demo mode enabled:

```bash
cargo install hotpath --features='tui,hotpath-alloc,demo' --version '^{{HOTPATH_VERSION}}'
```

Then launch the TUI:

```bash
hotpath
```

and you'll see timing, memory and other metrics.

Make sure to reinstall it without the auto-profiling features so that you can also observe metrics of other programs!

```bash
cargo install hotpath --features='tui' --version '^{{HOTPATH_VERSION}}'
```

## Learn more

Read the [complete guide to profiling Rust applications](/blog/profiling-rust-guide) - a comprehensive overview of debugging performance issues in Rust.

Explore the docs for customization options and advanced profiling features.

- [Profiling modes](./profiling_modes.html) - static reports vs live TUI dashboard
- [Profiling overhead](./profiling_overhead.html) - per-operation instrumentation cost and time sampling
- [Wall-clock time](./functions.html) - measure execution time of functions and code blocks
- [Memory profiling](./memory_profiling.html) - track allocations per function and thread, and process RSS
- [CPU profiling](./cpu_profiling.html) - attribute CPU samples to instrumented functions
- [Threads](./threads.html) - monitor threads usage
- [Async Data Flow](./data_flow.html) - monitor channels, streams, and futures
- [Locks](./locks.html) - track Mutex and RwLock wait and hold times
- [SQL queries](./sql_tracing.html) - profile query execution time for sqlx and Diesel
- [HTTP requests](./http_tracing.html) - profile reqwest and ureq client calls per endpoint
- [HTTP server (axum)](./axum_tracing.html) - profile axum request latency and errors per route
- [I/O tracing](./io_tracing.html) - monitor bytes, throughput and duration of TCP, Redis, and file operations
- [Tokio Runtime](./tokio_runtime.html) - monitor Tokio runtime worker stats and task scheduling
- [Debug & Metrics](./debug.html) - track custom values with dbg!, val!, and gauge! macros
- [Prometheus & Grafana](./prometheus_grafana.html) - export metrics to Prometheus and build Grafana dashboards
- [Criterion benchmarks](./criterion.html) - profile existing Criterion benchmarks with timing, allocations and Callgrind instruction counts
- [hotpath-cloud CI integration](./ci_integration.html) - performance regression detection on every pull request
- [Live MCP Server](./mcp.html) - LLM queries of a running application's profiling data via Model Context Protocol
- [hotpath-cloud MCP Server and REST API](./mcp_api.html) - AI assistant access to stored CI reports
- [Cargo flamegraph alternatives](/blog/sampling_comparison) - when to use `hotpath` vs sampling profilers like perf and samply
- [Rust performance regression testing](/blog/rust-performance-benchmarking) - why benchmarks are noisy on GitHub Actions and how deterministic metrics catch regressions
- [Configuration](./configuration.html) - explore all config options

## Building in public

[Follow @pawelurbanekcom on X](https://x.com/pawelurbanekcom) or <a href="https://eepurl.com/dhuFg5" target="_blank" rel="noopener noreferrer" data-newsletter-open>subscribe to the mailing list</a> for development updates, new releases and performance tips.
