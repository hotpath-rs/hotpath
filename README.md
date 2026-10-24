# <img src="media/hotpath-logo2.png" alt="hotpath-rs logo" width="80px" align="left"> hotpath - Rust Profiler for Easy Performance Debugging 
[![Latest Version](https://img.shields.io/crates/v/hotpath.svg)](https://crates.io/crates/hotpath) [![Downloads](https://img.shields.io/crates/d/hotpath?cacheSeconds=86400)](https://crates.io/crates/hotpath) [![hotpath-cloud](https://img.shields.io/endpoint?url=https://hotpath.rs/badge-cloud.json)](https://hotpath.rs/app/repos/hotpath-rs/hotpath) [![Sponsor](https://img.shields.io/badge/Sponsor-hotpath--rs-6f42c1)](https://hotpath.rs/sponsorship)

A simple Rust profiler that shows exactly why your code is slow.

Performance feedback for developers and coding agents. Find CPU, memory, async, SQL, I/O and contention bottlenecks with a single Rust profiler. Export metrics to Prometheus and Grafana when you need production monitoring.

Try the TUI demo via SSH - no installation required:

```
ssh demo.hotpath.rs
```

Or let your AI coding agent set up profiling, paste into Claude Code, Codex, OpenCode or Cursor:

```
Set up hotpath-rs profiling in this Rust project using the instructions at https://hotpath.rs/agent/setup
```

[hotpath-rs](https://hotpath.rs) is an easy-to-configure Rust performance profiling toolkit that shows exactly where your code spends time, burns CPU, and allocates memory.

It helps you distinguish between functions that are slow because they wait on I/O and those that are CPU-intensive. Instrument functions, channels, futures, streams, SQL queries, HTTP calls, and byte-level I/O to find bottlenecks and focus optimizations where they matter most. Get actionable insights into time, memory, and async data flow with minimal setup.

Explore the full documentation at [hotpath.rs](https://hotpath.rs). See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

You can use it to produce one-off performance (timing, memory or CPU) reports:

![hotpath alloc report](media/hotpath-alloc-report2.png)

correlate all performance signals in Grafana with minimal profiling overhead:

![Grafana dashboard built on hotpath-rs Prometheus metrics showing slowest SQL queries, per-function allocations and requests by route](docs/src/images/grafana-dashboard.png)

catch performance regressions on every pull request with a simple [CI integration](https://hotpath.rs/cloud):

![hotpath Cloud pull request comment showing wall time, allocation and RSS changes against the main branch](docs/src/images/cloud-pr-comment.webp)

inspect throughput and latency of network, file or compression I/O streams:

![hotpath-rs I/O profiling report showing per-stream read counts, bytes, transfer rate, average and P95 latency](media/io_metrics.png)

analyze SQL/HTTP calls performance with automatic source function attribution:

![hotpath-rs SQL query profiling report showing per-query call counts, source function attribution, average and P95 execution time](media/sql_metrics.png)

monitor throughput, performance and max queue depth of instrumented channels:

![hotpath-rs channel profiling report showing throughput, send-to-receive latency and max queue depth per channel](media/channel_metrics.png)

or use the live TUI dashboard to monitor real-time performance and async data flow metrics with debug info:

https://github.com/user-attachments/assets/2e890417-2b43-4b1b-8657-a5ef3b458153

## Features

- **Time, CPU & memory profiling** - identify expensive functions, allocation hotspots, and investigate memory leaks.
- **Async observability** - futures, channels and streams.
- **I/O monitoring** - bytes, throughput, latency of any sync or async IO stream like files, TCP, or compression.
- **SQL query profiling** - query performance metrics for sqlx and Diesel.
- **HTTP calls profiling** - per-endpoint latency and error metrics for reqwest and ureq.
- **HTTP server profiling** - per-route response time and error metrics for axum.
- **Concurrency metrics** - Mutex/RwLock wait time and contention.
- **Tokio runtime monitoring** - workers, scheduling and queues.
- **Prometheus & Grafana integration** - export profiling metrics to your existing dashboards.
- **Live TUI dashboard & static reports** - real-time or one-off analysis.
- **CI regression detection** - benchmark every PR automatically.
- **MCP server for AI agents** - query profiling data in real time.
- **Zero cost when disabled** - fully feature-gated.

## Getting Started

### AI Setup (Recommended)

The quickest way to set up hotpath is to let your own AI coding agent do it. Paste this prompt into Claude Code, Codex, OpenCode, Cursor or any other coding agent, inside your project repo:

```text
Set up hotpath-rs profiling in this Rust project using the instructions at https://hotpath.rs/agent/setup
```

The agent inspects your project, including workspaces and any existing hotpath setup, adds the feature-gated dependency, instruments `main` and a small starting set of hot functions, channels and locks, and verifies that everything compiles with profiling enabled and disabled. When the project has a workload that exits, it runs it and shows you the first report; it does not start long-running servers just to get one. It then explains how to run profiling and the live TUI. It finally asks once whether to optionally set up [hotpath Cloud](https://hotpath.rs/cloud) to benchmark pull requests in CI.

The prompt is a one-time project setup. The `hotpath-rs` [agent skill](skills/hotpath-rs/SKILL.md) is what stays: reusable hotpath knowledge your agent loads in every later session. To install it, for example in a repo that already uses hotpath, run:

```bash
npx skills add hotpath-rs/hotpath --skill hotpath-rs
```

The skill maps every hotpath task, from instrumenting a crate to fixing a regression flagged in CI, to the documentation page that covers it, so your agent fetches the current docs instead of relying on its memory. Your agent remains in control: you review and approve edits through its regular permission prompts.

### Manual installation

Add to your `Cargo.toml`:

```toml
[dependencies]
hotpath = "0.29"

[features]
hotpath = ["hotpath/hotpath"]
hotpath-cpu = ["hotpath", "hotpath/hotpath-cpu"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
hotpath-prometheus = ["hotpath", "hotpath/hotpath-prometheus"]
```

This config ensures that the lib has no compile time or runtime overhead unless explicitly enabled via one of the `hotpath` features. Every profiling feature also enables `hotpath`, so a single flag like `--features=hotpath-alloc` is enough. All the lib dependencies are optional (i.e. not compiled) and all macros are noop unless profiling is enabled.

#### Basic setup

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

Now, run your program with `hotpath` (and optionally `hotpath-alloc` features):

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
| docs_example::async_function | 1000  | 1.16 ms  | 1.24 ms  | 1.16 s   | 96.84%  |
+------------------------------+-------+----------+----------+----------+---------+
| custom_block                 | 1000  | 16.86 µs | 29.50 µs | 16.86 ms | 1.41%   |
+------------------------------+-------+----------+----------+----------+---------+
| docs_example::sync_function  | 1000  | 12.16 µs | 23.84 µs | 12.17 ms | 1.02%   |
+------------------------------+-------+----------+----------+----------+---------+

alloc-bytes - Exclusive allocation bytes by each function (excluding nested calls).
Total: 100.1 KB
+------------------------------+-------+---------+---------+---------+---------+
| Function                     | Calls | Avg     | P95     | Total   | % Total |
+------------------------------+-------+---------+---------+---------+---------+
| docs_example::main           | 1     | 88.4 KB | 88.4 KB | 88.4 KB | 88.29%  |
+------------------------------+-------+---------+---------+---------+---------+
| docs_example::sync_function  | 1000  | 12 B    | 12 B    | 11.7 KB | 11.71%  |
+------------------------------+-------+---------+---------+---------+---------+
| custom_block                 | 1000  | 0 B     | 0 B     | 0 B     | 0.00%   |
+------------------------------+-------+---------+---------+---------+---------+
| docs_example::async_function | 1000  | 0 B     | 0 B     | 0 B     | 0.00%   |
+------------------------------+-------+---------+---------+---------+---------+

threads - Thread CPU and memory statistics. (Max RSS: 8.6 MB, Alloc: 108.5 KB, Dealloc: 123.8 KB, Diff: -15.3 KB, 5/15)
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
| hp-functions    | 1.3% | 1.0% | 0 B      | 2.0 KB  | -2.0 KB  |
+-----------------+------+------+----------+---------+----------+
```

## Full documentation

See the full docs and advanced config tutorials at [hotpath.rs](https://hotpath.rs).

Read the [complete guide to profiling Rust applications](https://hotpath.rs/blog/profiling-rust-guide) - a comprehensive overview of debugging performance issues in Rust.

- [Profiling modes](https://hotpath.rs/profiling_modes) - static reports vs live TUI dashboard
- [Profiling overhead](https://hotpath.rs/profiling_overhead) - per-operation instrumentation cost and time sampling
- [Functions](https://hotpath.rs/functions) - measure execution time and memory allocations
- [CPU profiling](https://hotpath.rs/cpu_profiling) - attribute CPU samples to instrumented functions
- [Threads](https://hotpath.rs/threads) - monitor threads usage
- [Async Data Flow](https://hotpath.rs/data_flow) - monitor channels, streams, and futures
- [Locks](https://hotpath.rs/locks) - track Mutex and RwLock wait and hold times
- [SQL queries](https://hotpath.rs/sql_tracing) - profile query execution time for sqlx and Diesel
- [HTTP requests](https://hotpath.rs/http_tracing) - profile reqwest and ureq client calls per endpoint
- [HTTP server (axum)](https://hotpath.rs/axum_tracing) - profile axum request latency and errors per route
- [I/O tracing](https://hotpath.rs/io_tracing) - monitor bytes, throughput and duration of TCP, Redis, and file operations
- [Tokio Runtime](https://hotpath.rs/tokio_runtime) - monitor Tokio runtime worker stats and task scheduling
- [Debug & Metrics](https://hotpath.rs/debug) - track custom values with dbg!, val!, and gauge! macros
- [Prometheus & Grafana](https://hotpath.rs/prometheus_grafana) - export metrics to Prometheus and build Grafana dashboards
- [hotpath Cloud CI integration](https://hotpath.rs/ci_integration) - performance regression detection on every pull request
- [MCP Server](https://hotpath.rs/mcp) - LLM integration via Model Context Protocol
- [Cargo flamegraph alternatives](https://hotpath.rs/blog/sampling_comparison) - when to use `hotpath` vs sampling profilers like perf and samply
- [Configuration](https://hotpath.rs/configuration) - explore all config options

## hotpath Cloud

My long-term goal for hotpath-rs is to become the single place to understand all performance signals in a Rust application. From CPU and memory usage to locks, channels, and async execution, all the way up to SQL queries and HTTP/RPC calls.

I'm also building a hosted version that makes profiling reports easier to share, compare, and analyze across pull requests, deployments, and teams. Read more: https://hotpath.rs/cloud

## Status

This project is under active development. Core public APIs are stable, but implementation details (JSON report formats, TUI/MCP internals, and advanced config options) may change between releases as the project evolves.
