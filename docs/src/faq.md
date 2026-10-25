# FAQ

Frequently asked questions about `hotpath-rs`.

## What is the overhead of profiling with hotpath?

With the `hotpath` Cargo feature disabled, there is no overhead at all. Every macro compiles to a no-op, and none of the library dependencies are built.

With the feature enabled, the overhead is a few tens of nanoseconds per instrumented operation. For example, `#[hotpath::measure]` adds ~40 ns per function call, and lock and channel wrappers add 30-90 ns per operation. Events are recorded into lock-free per-thread queues and processed by background workers, so the instrumented code never waits on a lock.

For very hot code paths, you can lower the cost further with [time sampling](profiling_overhead.md#reducing-overhead-time-sampling). It measures durations for only a fraction of calls and keeps the call counts exact. See [Profiling overhead](profiling_overhead.md) for full benchmark numbers and methodology.

## What types of metrics can be profiled?

`hotpath-rs` can profile:

- [Functions](functions.md) - execution time, call counts, percentiles, and memory allocations of sync and async functions.
- [CPU usage](cpu_profiling.md) - sampling-based CPU profiling powered by samply.
- [Instruction counts](callgrind.md) - deterministic per-function instruction counts with Valgrind's Callgrind, for low-noise regression checks in CI.
- [Threads](threads.md) - per-thread CPU usage and memory statistics.
- [Async data flow](data_flow.md) - channel throughput and queue depth, streams, and futures.
- [Locks](locks.md) - wait and hold times of mutexes and read-write locks.
- [SQL queries](sql_tracing.md) - query timing for sqlx, Diesel, and Toasty.
- [HTTP requests](http_tracing.md) - outgoing requests made with reqwest or ureq, grouped by endpoint.
- [HTTP server routes](axum_tracing.md) - response times and error counts per axum route.
- [I/O](io_tracing.md) - operation counts, bytes, transfer rates, and latency of files, sockets, and other streams.
- [Tokio runtime](tokio_runtime.md) - worker thread utilization, task scheduling, and queue depths.

All of these metrics can also be exported to [Prometheus and Grafana](prometheus_grafana.md).

## How do I profile async Rust and Tokio applications?

Add `#[hotpath::measure]` to an `async fn` to record its wall-clock time and allocations, just like a sync function. Use `#[hotpath::measure(future = true)]` to also track the future itself: poll counts, poll durations, and whether each call completed or was cancelled. See [Async functions](functions.md#async-functions-and-future--true) for details.

Async function profiling works on any async runtime, including Tokio and smol, with no runtime-specific feature flag. Enable the `tokio` feature only for Tokio-specific integrations:

- [Tokio runtime metrics](tokio_runtime.md) - call `hotpath::tokio_runtime!()` in your async main to monitor worker thread utilization, task scheduling, and queue depths.
- [Channels](data_flow.md#channels-monitoring) - wrap Tokio channels with `channel!` to track throughput and queue depth.
- [Locks](locks.md) - wrap `tokio::sync::Mutex` and `RwLock` to measure wait and hold times.

You can also monitor [streams and futures](data_flow.md) with the `stream!` and `future!` macros.

## How do I profile memory allocations?

Enable the `hotpath-alloc` feature:

```bash
cargo run --features='hotpath-alloc'
```

`hotpath` then installs a tracking global allocator and reports allocation counts and bytes for every instrumented function. Async functions are measured per poll, so allocations are attributed correctly even when a future moves between Tokio worker threads.

By default, each function reports only its own allocations, excluding nested instrumented calls. Set `HOTPATH_ALLOC_CUMULATIVE=true` to include them. If your program uses a custom allocator like jemalloc or mimalloc, pass it with `#[hotpath::main(allocator = ...)]`. See [Memory and allocations profiling](functions.md#memory-and-allocations-profiling) for details.

The [threads view](threads.md) complements per-function stats with per-thread allocated and deallocated bytes and process RSS. It's useful for spotting memory leaks in long-running applications.

## Is it safe to profile production applications?

Yes. Every instrumented subsystem is covered by an integration test suite, and the profiling pipeline is designed to keep a low and predictable overhead.

To profile production safely, follow these guidelines:

- **Run your test suite with profiling enabled.** Use `cargo test --features=hotpath` to confirm that your application behaves the same with instrumentation turned on.
- **Don't instrument very short, very hot functions.** A function that runs in the low microseconds and is called millions of times contributes a large share of the total processing time, so even ~40 ns per call can add up. Instrument larger units of work instead, or use [time sampling](profiling_overhead.md#reducing-overhead-time-sampling) to reduce the cost.
- **Review the metrics server.** By default, a profiled application exposes an HTTP metrics server on `localhost:6770`, which feeds the TUI console. Set `HOTPATH_METRICS_SERVER_OFF=true` to disable it, `HOTPATH_METRICS_PORT` to change the port, or `HOTPATH_METRICS_AUTH_TOKEN` to require a token on every request. See [Configuration](configuration.md) for all options.

## How is hotpath-rs different from Criterion.rs?

[Criterion.rs](https://github.com/criterion-rs/criterion.rs) is a statistics-driven benchmarking library for Rust. Criterion.rs focuses on statistically rigorous benchmarking: _"is this faster?"_. hotpath-rs focuses on profiling instrumented application code: _"where does my program spend its time, memory, and queries?"_.

**Criterion.rs** runs the code you choose in a dedicated benchmark harness (`cargo bench`), many times over, and applies statistical analysis to the results. Its strengths:

- Precise measurements with warmup, outlier detection, and confidence intervals.
- Detects whether, and by how much, performance changed since the previous run.
- Benchmarks anything from a single function to larger workloads, including [async code](https://bheisler.github.io/criterion.rs/book/user_guide/benchmarking_async.html) on Tokio, smol, and other executors.
- HTML reports and plots, runs on stable Rust, and benchmarks live in `benches/`, so your application code stays untouched.

Criterion.rs measures the workload as a whole. It doesn't automatically provide application-wide telemetry, such as which SQL queries ran, how channel queues grew, or which locks were contended. By default it measures wall-clock time, so results on shared CI runners are affected by CPU frequency scaling, scheduling, and other workloads.

**hotpath-rs** instruments the real program while it runs a real workload. Its strengths:

- Profiles many functions at once and shows which of them dominate the total execution time.
- Collects signals beyond timing: memory allocations, [SQL queries](sql_tracing.md), [HTTP calls](http_tracing.md), [lock contention](locks.md), [channel queues](data_flow.md#channels-monitoring), and [I/O](io_tracing.md). Counts like executed queries or allocated bytes usually stay the same between runs of the same workload, even when timings are noisy.
- Reproducible instruction counts in CI. Under [Valgrind's Callgrind](callgrind.md), the `hotpath-callgrind` feature reports instructions executed per function instead of elapsed time; in CI, a merged report carries both, the native run's timing next to the counts. With a deterministic workload and the same Valgrind version, Rust toolchain, and target, counts repeat closely between runs, so small changes stay visible on shared GitHub Actions runners.

Its trade-offs are a small per-call [overhead](profiling_overhead.md) and no built-in statistical analysis of timings. Callgrind runs only on x86_64 Linux and slows the program down substantially. Instruction counts track CPU work, not time spent waiting on I/O or locks, and fewer instructions don't guarantee faster execution.

If you prefer instruction counts in a dedicated benchmark harness, [Gungraun](https://github.com/gungraun/gungraun) (formerly Iai-Callgrind) is an established Valgrind-based benchmarking framework for Rust.

The tools complement each other. Use hotpath-rs to find the bottleneck in the real application, then move the hot code into a Criterion.rs benchmark to iterate on its optimization with precise measurements.

## How is hotpath-rs different from hyperfine?

[hyperfine](https://github.com/sharkdp/hyperfine) is a command-line benchmarking tool that measures the performance of whole commands.

**hyperfine** treats the program as a black box. Its strengths:

- Works with any command or binary, in any language, with no code changes.
- Runs each command multiple times with statistical analysis and outlier detection, and supports warmup runs and `--prepare` commands for cold-cache benchmarks.
- Compares several commands side by side, scans over parameters, and exports results to JSON, CSV, and Markdown.
- Since version 2.0, reports peak memory usage alongside wall-clock time by default, and can optionally measure hardware performance counters such as CPU cycles and instructions, where the platform supports them.

hyperfine measures whole-command performance and doesn't attribute costs to the functions, queries, or locks inside the program.

**hotpath-rs** looks inside the program. It reports per-function timing and allocations, along with SQL queries, HTTP calls, locks, channels, and I/O, so you can see where the time is spent and what changed between two versions. In CI, it can also [count executed instructions](callgrind.md) per function under Callgrind, without hardware performance counters. In exchange, it requires adding instrumentation macros to your code, works only for Rust programs, and adds a small [overhead](profiling_overhead.md) to every instrumented call.

hyperfine is a great fit for end-to-end benchmarks of CLI tools and whole programs, and hotpath-rs helps explain the numbers it produces. The [allocation overhead benchmarks](/blog/rust-performance-profiling#alloc-metrics-cpu-cache-busting) of hotpath-rs itself are timed with hyperfine.

## How is hotpath-rs different from tokio-console?

[tokio-console](https://github.com/tokio-rs/console) is the official diagnostics and debugging tool for async Rust applications running on Tokio. The two tools answer different questions, and they work well side by side.

**tokio-console** looks at the runtime from the inside. It shows the lifecycle of individual Tokio tasks, with poll counts and busy, scheduled, and idle time, plus waker activity and Tokio resources such as timers, semaphores, and `tokio::sync` primitives. It also warns about patterns like lost wakers, excessive self-wakes, and tasks that never yield. It's an interactive tool built for live inspection, and it requires the [`console-subscriber`](https://docs.rs/console-subscriber) crate, building with `--cfg tokio_unstable`, and the `tokio-console` client connected to the running process.

**hotpath-rs** looks at application-level performance. It measures function timings and allocations, channel throughput and queue depth, lock wait and hold times, poll counts and lifecycle of instrumented futures and streams, as well as SQL queries, HTTP calls, and I/O. Channel and lock instrumentation isn't limited to Tokio, and also covers `std`, `crossbeam`, `flume`, and others. Async function profiling works on any runtime, and [Tokio runtime metrics](tokio_runtime.md) add worker and queue statistics. It's enabled with a cargo feature and is a no-op when the feature is disabled. It prints a static report on exit, can be monitored live in the [TUI console](profiling_modes.md#live-tui-dashboard), and its JSON reports can be compared between commits in [CI](ci_integration.md). It only sees the code you instrument, and it doesn't provide tokio-console's per-task lifecycle and waker diagnostics.

Some examples of which tool answers which question:

| Question | Tool |
|---|---|
| Which task never gets woken up again? | tokio-console |
| Which task blocks a worker thread with a long poll? | tokio-console |
| Which channel's queue keeps growing, and how long do messages wait? | hotpath-rs |
| Which lock do threads wait on the longest, and how long is it held? | hotpath-rs |
| Did this PR make an endpoint slower or add SQL queries? | hotpath-rs |

Use tokio-console when you suspect the runtime itself (starved or stuck tasks, misbehaving wakers), and use hotpath-rs to find out which part of your application code is slow, congested, or contended. Both can run in the same application.
