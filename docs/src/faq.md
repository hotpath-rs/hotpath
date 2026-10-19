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
- [Threads](threads.md) - per-thread CPU usage and memory statistics.
- [Async data flow](data_flow.md) - channel throughput and queue depth, streams, and futures.
- [Locks](locks.md) - wait and hold times of mutexes and read-write locks.
- [SQL queries](sql_tracing.md) - query timing for sqlx, Diesel, and Toasty.
- [HTTP requests](http_tracing.md) - outgoing requests made with reqwest or ureq, grouped by endpoint.
- [HTTP server routes](axum_tracing.md) - response times and error counts per axum route.
- [I/O](io_tracing.md) - operation counts, bytes, transfer rates, and latency of files, sockets, and other streams.
- [Tokio runtime](tokio_runtime.md) - worker thread utilization, task scheduling, and queue depths.

All of these metrics can also be exported to [Prometheus and Grafana](prometheus_grafana.md).

## Is it safe to profile production applications?

Yes. Every instrumented subsystem is covered by an integration test suite, and the profiling pipeline is designed to keep a low and predictable overhead.

To profile production safely, follow these guidelines:

- **Run your test suite with profiling enabled.** Use `cargo test --features=hotpath` to confirm that your application behaves the same with instrumentation turned on.
- **Don't instrument very short, very hot functions.** A function that runs in the low microseconds and is called millions of times contributes a large share of the total processing time, so even ~40 ns per call can add up. Instrument larger units of work instead, or use [time sampling](profiling_overhead.md#reducing-overhead-time-sampling) to reduce the cost.
