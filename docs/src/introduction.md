# A simple Rust profiler that shows exactly why your code is slow

<h2 class="hero-subtitle"><b>Performance profiling for developers and coding agents.</b> Find CPU, memory, async, SQL, I/O and contention bottlenecks with a single Rust profiler. Export metrics to Prometheus and Grafana when you need production monitoring.</h2>

<div class="hero-badges">
  <a href="https://github.com/hotpath-rs/hotpath" target="_blank" rel="noopener noreferrer"><img src="{{#asset-hash images/stars-hotpath-rs-hotpath.svg}}" alt="GitHub Stars"></a>
  <a href="https://crates.io/crates/hotpath" target="_blank" rel="noopener noreferrer"><img src="https://img.shields.io/crates/d/hotpath?cacheSeconds=86400" alt="crates.io downloads"></a>
  <a href="https://hotpath.rs/app/repos/hotpath-rs/hotpath" target="_blank" rel="noopener noreferrer"><img src="https://img.shields.io/endpoint?url=https://hotpath.rs/badge.json" alt="hotpath"></a>
</div>

<div class="hero-row">
  <img src="{{#asset-hash images/hotpath-ferris.webp}}" alt="hotpath-rs Rust profiling mascot Ferris the crab" class="ferris-img-hero">
  <div class="ssh-demo-container">
    <p class="ssh-demo-label">Try the TUI demo via SSH - no installation required:</p>
    <div class="terminal-shell">
      <span class="terminal-prompt">$</span>
      <span class="terminal-command">ssh demo.hotpath.rs</span>
    </div>
    <p class="ssh-demo-label">Or let your own AI agent configure profiling in a repo:</p>
    <div class="terminal-shell terminal-shell-multi">
      <div class="terminal-tabs">
        <button class="terminal-tab active" data-agent="claude&nbsp;&nbsp;" onclick="hotpathInitAgent('claude&nbsp;&nbsp;')">Claude</button>
        <button class="terminal-tab" data-agent="codex&nbsp;&nbsp;&nbsp;" onclick="hotpathInitAgent('codex&nbsp;&nbsp;&nbsp;')">Codex</button>
        <button class="terminal-tab" data-agent="opencode" onclick="hotpathInitAgent('opencode')">OpenCode</button>
      </div>
      <div class="terminal-line">
        <span class="terminal-prompt">$</span>
        <span class="terminal-command">cargo install hotpath</span>
      </div>
      <div class="terminal-line">
        <span class="terminal-prompt">$</span>
        <span class="terminal-command">hotpath init --agent <span id="init-agent-name">claude&nbsp;&nbsp;</span></span>
      </div>
    </div>
  </div>
</div>

<script>
function hotpathInitAgent(agent) {
  document.getElementById('init-agent-name').textContent = agent;
  document.querySelectorAll('.terminal-tab').forEach(function (tab) {
    tab.classList.toggle('active', tab.dataset.agent === agent);
  });
}
</script>

[hotpath-rs](https://github.com/hotpath-rs/hotpath) is an easy-to-configure Rust performance profiling toolkit that shows exactly where your code spends time, burns CPU, and allocates memory. 

It helps you distinguish between functions that are slow because they wait on I/O and those that are CPU-intensive. Instrument functions, channels, futures, streams, SQL queries, HTTP calls, and byte-level I/O to find bottlenecks and focus optimizations where they matter most. Get actionable insights into time, memory, and async data flow with minimal setup.

<div style="clear: both;"></div>

<div class="trusted-by">
  <p class="trusted-by-tagline">Used by <a href="/adoption">{{#adoption_count}} open-source projects</a>, including:</p>
  <div class="trusted-by-grid">
    <a href="https://github.com/PostHog/posthog" target="_blank" rel="noopener noreferrer" class="trusted-by-project">
      <span class="trusted-by-name">PostHog/posthog</span>
      <img src="{{#asset-hash images/stars-PostHog-posthog.svg}}" alt="posthog GitHub stars">
    </a>
    <a href="https://github.com/rustfs/rustfs" target="_blank" rel="noopener noreferrer" class="trusted-by-project">
      <span class="trusted-by-name">rustfs/rustfs</span>
      <img src="{{#asset-hash images/stars-rustfs-rustfs.svg}}" alt="rustfs GitHub stars">
    </a>
    <a href="https://github.com/apache/opendal" target="_blank" rel="noopener noreferrer" class="trusted-by-project">
      <span class="trusted-by-name">apache/opendal</span>
      <img src="{{#asset-hash images/stars-apache-opendal.svg}}" alt="opendal GitHub stars">
    </a>
    <a href="https://github.com/maplibre/martin" target="_blank" rel="noopener noreferrer" class="trusted-by-project">
      <span class="trusted-by-name">maplibre/martin</span>
      <img src="{{#asset-hash images/stars-maplibre-martin.svg}}" alt="martin GitHub stars">
    </a>
    <a href="https://github.com/marc2332/freya" target="_blank" rel="noopener noreferrer" class="trusted-by-project">
      <span class="trusted-by-name">marc2332/freya</span>
      <img src="{{#asset-hash images/stars-marc2332-freya.svg}}" alt="freya GitHub stars">
    </a>
    <a href="https://github.com/parseablehq/parseable" target="_blank" rel="noopener noreferrer" class="trusted-by-project">
      <span class="trusted-by-name">parseablehq/parseable</span>
      <img src="{{#asset-hash images/stars-parseablehq-parseable.svg}}" alt="parseable GitHub stars">
    </a>
  </div>
</div>

You can use it to produce one-off performance (timing, memory or CPU) reports:

<img loading="lazy" src="{{#asset-hash images/hotpath-alloc-report.png}}" alt="hotpath-rs memory allocation profiling report showing per-function byte counts">

correlate all performance signals in Grafana with minimal profiling overhead:

<img loading="lazy" src="{{#asset-hash images/grafana-dashboard.png}}" alt="Grafana dashboard built on hotpath-rs Prometheus metrics showing slowest SQL queries, per-function allocations and requests by route">

catch performance regressions on every pull request with a simple [CI integration](/cloud):

<img loading="lazy" src="{{#asset-hash images/cloud-pr-comment.webp}}" alt="hotpath Cloud pull request comment showing wall time, allocation and RSS changes against the main branch">

inspect throughput and latency of network, file or compression I/O streams:

<img loading="lazy" src="{{#asset-hash images/io_metrics.png}}" alt="hotpath-rs I/O profiling report showing per-stream read counts, bytes, transfer rate, average and P95 latency">

analyze SQL/HTTP calls performance with automatic source function attribution:

<img loading="lazy" src="{{#asset-hash images/sql_metrics.png}}" alt="hotpath-rs SQL query profiling report showing per-query call counts, source function attribution, average and P95 execution time">

monitor throughput, performance and max queue depth of instrumented channels:

<img loading="lazy" src="{{#asset-hash images/channel_metrics.png}}" alt="hotpath-rs channel profiling report showing throughput, send-to-receive latency and max queue depth per channel">

or use the live TUI dashboard to monitor real-time performance and async data flow metrics with debug info:

<video loading="lazy" width="100%" loop muted playsinline controls poster="{{#asset-hash images/hotpath-live-dashboard-poster.jpg}}">
  <source src="{{#asset-hash videos/hotpath-live-dashboard.mp4}}" type="video/mp4">
</video>

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

The quickest way to set up hotpath is to let your own AI coding agent do it. Install the `hotpath` CLI and run `init` inside your project repo:

```bash
cargo install hotpath --version '^{{HOTPATH_VERSION}}'
hotpath init --agent claude # or --agent codex / --agent opencode
```

`hotpath init` downloads the [hotpath_init agent skill](https://github.com/hotpath-rs/hotpath/blob/main/skills/hotpath_init/SKILL.md) from GitHub and starts your installed Claude Code, Codex or OpenCode with it as setup instructions. The agent inspects your project, adds the dependency, instruments `main` and a starting set of functions, channels and locks, then verifies that everything compiles with profiling enabled and disabled.

Your agent remains in control: you review and approve edits through its regular permission prompts. Requires `curl` and the `claude`, `codex` or `opencode` CLI on `PATH`.

### Manual installation

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

### Basic setup

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
- [Functions](./functions.html) - measure execution time and memory allocations
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
- [hotpath Cloud CI integration](./ci_integration.html) - performance regression detection on every pull request
- [MCP Server](./mcp.html) - LLM integration via Model Context Protocol
- [Cargo flamegraph alternatives](/blog/sampling_comparison) - when to use `hotpath` vs sampling profilers like perf and samply
- [Configuration](./configuration.html) - explore all config options
