# A simple Rust profiler that shows exactly why your code is slow

<h2 class="hero-subtitle"><b>Performance profiling for developers and coding agents.</b> Find CPU, memory, async, SQL, I/O and contention bottlenecks with a single Rust profiler. Export metrics to Prometheus and Grafana when you need production monitoring.</h2>

<div class="hero-badges">
  <a href="https://github.com/hotpath-rs/hotpath" target="_blank" rel="noopener noreferrer"><img src="{{#asset-hash images/stars-hotpath-rs-hotpath.svg}}" alt="GitHub Stars"></a>
  <a href="https://crates.io/crates/hotpath" target="_blank" rel="noopener noreferrer"><img src="https://img.shields.io/crates/d/hotpath?cacheSeconds=86400" alt="crates.io downloads"></a>
  <a href="https://hotpath.rs/app/repos/hotpath-rs/hotpath" target="_blank" rel="noopener noreferrer"><img src="https://img.shields.io/endpoint?url=https://hotpath.rs/badge-cloud.json" alt="hotpath-cloud"></a>
</div>

<div class="hero-row">
  <img src="{{#asset-hash images/hotpath-ferris.webp}}" alt="hotpath-rs Rust profiling mascot Ferris the crab" class="ferris-img-hero">
  <div class="ssh-demo-container">
    <p class="ssh-demo-label">Try the TUI demo via SSH - no installation required:</p>
    <button type="button" class="terminal-shell terminal-copyable copy-tooltip" data-tooltip-copied="Copied" onclick="hotpathCopy(this)" title="Click to copy">
      <span class="terminal-prompt">$</span>
      <span class="terminal-command">ssh demo.hotpath.rs</span>
    </button>
    <p class="ssh-demo-label">Or let your AI coding agent set up profiling:</p>
    <button type="button" class="agent-pill copy-tooltip" data-tooltip="Copies a setup prompt for your AI coding agent" data-tooltip-copied="Copied! Paste into your AI coding agent." data-copy="Set up hotpath-rs profiling in this Rust project using the instructions at https://hotpath.rs/agent/setup" onclick="hotpathCopy(this)">
      <svg class="agent-pill-copy" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1"/></svg>
      <span>Set up hotpath-rs with AI</span>
      <span class="agent-logos">
        <svg class="agent-logo agent-logo-claude" viewBox="0 0 24 24" role="img" aria-label="Claude Code"><path d="m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z"/></svg>
        <svg class="agent-logo" viewBox="0 0 24 24" role="img" aria-label="Codex"><path d="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z"/></svg>
        <svg class="agent-logo" viewBox="0 0 24 24" role="img" aria-label="Cursor"><path d="M11.503.131 1.891 5.678a.84.84 0 0 0-.42.726v11.188c0 .3.162.575.42.724l9.609 5.55a1 1 0 0 0 .998 0l9.61-5.55a.84.84 0 0 0 .42-.724V6.404a.84.84 0 0 0-.42-.726L12.497.131a1.01 1.01 0 0 0-.996 0M2.657 6.338h18.55c.263 0 .43.287.297.515L12.23 22.918c-.062.107-.229.064-.229-.06V12.335a.59.59 0 0 0-.295-.51l-9.11-5.257c-.109-.063-.064-.23.061-.23"/></svg>
      </span>
      <span class="agent-pill-status" role="status"></span>
    </button>
  </div>
</div>

<script>
function hotpathCopy(button) {
  var text = button.dataset.copy || button.querySelector('.terminal-command').textContent;
  var status = button.querySelector('[role="status"]');
  navigator.clipboard.writeText(text).then(function () {
    button.classList.add('copied');
    if (status) status.textContent = button.dataset.tooltipCopied;
    button.addEventListener('mouseleave', function () {
      button.classList.remove('copied');
      if (status) status.textContent = '';
    }, { once: true });
  });
}
</script>

[hotpath-rs](https://github.com/hotpath-rs/hotpath) is an easy-to-configure Rust performance profiling toolkit that shows exactly where your code spends time, burns CPU, and allocates memory. 

It helps you distinguish between functions that are slow because they wait on I/O and those that are CPU-intensive. Instrument functions, channels, futures, streams, SQL queries, HTTP calls, and byte-level I/O to find bottlenecks and focus optimizations where they matter most. Get actionable insights into time, memory, and async data flow with minimal setup. For noise-free CI benchmarks, it can also count executed CPU instructions with [Callgrind](callgrind.md) instead of measuring wall-clock time.

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

<a id="tui"></a>or use the live TUI dashboard to monitor real-time performance and async data flow metrics with debug info:

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
- **[Deterministic instruction counts](callgrind.md)** - Callgrind-based CPU measurements, stable across noisy CI runners.
- **MCP server for AI agents** - query profiling data in real time.
- **Zero cost when disabled** - fully feature-gated.

## Getting Started

### AI Setup (Recommended)

The quickest way to set up hotpath is to let your own AI coding agent do it. Paste this prompt into Claude Code, Codex, OpenCode, Cursor or any other coding agent, inside your project repo:

```text
Set up hotpath-rs profiling in this Rust project using the instructions at https://hotpath.rs/agent/setup
```

The agent inspects your project, including workspaces and any existing hotpath setup, adds the feature-gated dependency, instruments `main` and a small starting set of hot functions, channels and locks, and verifies that everything compiles with profiling enabled and disabled. When the project has a workload that exits, it runs it and shows you the first report. It then explains how to run profiling and the live TUI. It finally asks once whether to optionally set up [hotpath Cloud](cloud.md) to benchmark pull requests in CI.

The prompt is a one-time project setup. The `hotpath-rs` [agent skill](https://github.com/hotpath-rs/hotpath/blob/main/skills/hotpath-rs/SKILL.md) is what stays: reusable hotpath knowledge your agent loads in every later session. To install it, for example in a repo that already uses hotpath, run:

```bash
npx skills add hotpath-rs/hotpath --skill hotpath-rs
```

The skill maps every hotpath task, from instrumenting a crate to fixing a regression flagged in CI, to the documentation page that covers it, so your agent fetches the current docs instead of relying on its memory. Your agent remains in control: you review and approve edits through its regular permission prompts.

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
