# Rust Threads performance monitoring: CPU and memory metrics 

The threads view shows live per-thread CPU and memory metrics for the instrumented process. Reach for it when you need to answer questions that function-level profiling can't: which thread is burning CPU, which thread's allocations keep growing, or which threads sit blocked while the rest of your Tokio app starves. It works on Linux, macOS and Windows, and it's enabled by default via the `threads` feature flag, so if you already use hotpath for [time, CPU and memory profiling](./introduction.md), per-thread monitoring is active out of the box.

## Enabling per-thread monitoring

Add hotpath to your `Cargo.toml` behind feature flags:

```toml
[dependencies]
hotpath = "{{HOTPATH_VERSION}}"

[features]
hotpath = ["hotpath/hotpath"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
```

and attach the `#[hotpath::main]` macro to your entry point:

```rust
#[hotpath::main]
fn main() {
    // your code
}
```

Then run with profiling enabled:

```bash
cargo run --features='hotpath-alloc'
```

A few things to know:

- `threads` is a default feature of the hotpath crate, so no extra flag is needed - thread monitoring runs whenever the `hotpath` feature is on.
- The per-thread **Alloc / Dealloc / Diff** columns require the `hotpath-alloc` feature. Without it you still get the CPU metrics.
- There is zero compile-time or runtime overhead when the `hotpath` feature is disabled - all macros become noops and dependencies are not compiled.
- hotpath's own `hp-*` threads (metrics server, background workers) are hidden from the thread list; set `HOTPATH_SHOW_INTERNAL_THREADS=1` to see them.
- The sampling interval defaults to 250ms and is tunable via `HOTPATH_THREADS_INTERVAL_MS`. The number of rows in the final report is capped by `HOTPATH_THREADS_LIMIT` (default: 5).

## Metrics reference

The view header shows process-wide numbers (the PID of the profiled process is in the TUI top bar, use it to correlate hotpath's view with `ps`, `top` or `htop`):

**Alloc / Dealloc / Diff** - total bytes allocated and deallocated across all threads, and the difference between the two. If Diff keeps growing while your app is under steady-state load, memory is being retained somewhere - a leak signal worth chasing down. Requires the `hotpath-alloc` feature.

**RSS** - Resident Set Size, the physical memory the process currently occupies. RSS includes code, thread stacks and allocator slack, so it can stay flat while the allocation Diff grows (the allocator reuses freed pages) or grow while Diff stays flat. Comparing the two tells you whether memory growth comes from your allocations or from elsewhere.

**Max RSS** - the highest RSS the kernel has recorded for the program since it started, so a short spike between two refreshes is never missed. On Linux it covers the program itself only: memory used by a launcher that exec'd into it, such as `cargo run` building the binary first, is not included. If `/proc/self/status` can't be read (some sandboxes), hotpath falls back to `getrusage`, which may include that launcher memory. On macOS Max RSS comes from `getrusage`, and on Windows from the process's peak working set. The TUI shows both; the final report keeps only Max RSS, since a snapshot taken at exit says little about the run.

And per-thread metrics:

**Thread** - the logical name set via `std::thread::Builder::name` or by the runtime (e.g. `tokio-runtime-w`). Unnamed threads show up as `thread_N`, so naming the threads you spawn makes this view far more useful.

**TID** - the OS thread identifier. It matches what `htop -H`, `gdb` and sampling profilers report, so you can cross-reference the same thread across tools.

**Status** - the current execution state followed by the raw OS state code, shown in the live TUI and the JSON API (a point-in-time value, so the final report table omits it). `Running` means on-CPU right now. `Sleeping` means parked or waiting - completely normal for idle Tokio workers. `Blocked` means an uninterruptible wait, usually disk I/O; a thread that is persistently `Blocked` is doing synchronous I/O that stalls it, which is especially bad inside async worker threads.

**CPU %** - instantaneous CPU utilization, computed from deltas of cumulative CPU time between 250ms samples. A worker pinned near 100% indicates a busy loop or heavy computation. Like Status it is a point-in-time value, so it appears in the TUI and the JSON API only.

**Max %** (`Max%` in the final report) - the peak CPU utilization ever observed for the thread, so short spikes don't disappear between refreshes.

**Avg %** (`Avg%` in the final report) - lifetime average CPU utilization: total CPU time consumed by the thread divided by the profiler's elapsed time. Max% and Avg% together summarize a thread's whole run, which is why they are the two CPU columns kept in the final report.

**Alloc / Dealloc** - total bytes allocated and deallocated, attributed to the thread that performed them. Requires the `hotpath-alloc` feature.

**Diff** - Alloc minus Dealloc for that thread. A Diff that keeps climbing on a long-running thread is the per-thread leak signal; jump to the [functions allocation view](./functions.html) to find which function is responsible.

## Reading the output

The final report prints a threads section like this:

```
threads - Thread CPU and memory statistics. (Max RSS: 7.8 MB, Alloc: 2.1 MB, Dealloc: 304.3 KB, Diff: 1.8 MB)
+----------+-------+-------+----------+---------+----------+
| Thread   | Max%  | Avg%  | Alloc    | Dealloc | Diff     |
+----------+-------+-------+----------+---------+----------+
| main     | 6.3%  | 5.1%  | 367.8 KB | 9.9 KB  | 357.9 KB |
+----------+-------+-------+----------+---------+----------+
| worker-1 | 99.0% | 97.4% | 12.4 KB  | 12.1 KB | 316 B    |
+----------+-------+-------+----------+---------+----------+
| thread_5 | -     | -     | 640 B    | 24 B    | 616 B    |
+----------+-------+-------+----------+---------+----------+
```

Two things stand out here. `main` barely uses CPU (5.1% on average) yet its Diff is 357.9 KB and growing - allocations made on that thread are being retained, so whatever it built up is never freed. `worker-1` averages 97.4% CPU for the whole run - that's the compute hotspot, and since its Diff is near zero, it's CPU-bound rather than allocation-heavy. The final report keeps the two aggregate CPU columns (Max% and Avg%); the instantaneous CPU% and thread Status live in the [TUI](./profiling_modes.md#live-tui-dashboard) and the JSON API, where a point-in-time value is meaningful. This kind of per-thread visibility matters most in latency-critical apps, where one saturated thread can delay everything behind it.

The live TUI shows the same data refreshing in real time:

<img loading="lazy" src="{{#asset-hash images/threads-view.png}}" alt="hotpath-rs TUI showing per-thread CPU and memory usage monitoring">

## FAQ

**How do I find which thread is using the most CPU in Rust?**

In the final report, sort by Avg% for sustained load and check Max% for spikes that a single sample would miss. In the live TUI you also get the instantaneous CPU% and the thread's Status. To attribute that CPU time to specific functions, use [CPU profiling](./cpu_profiling.html).

**How do I detect a memory leak in a specific thread?**

Run with the `hotpath-alloc` feature and watch the Diff column over time. A thread whose Diff grows steadily under constant load is retaining memory. Then switch to the [functions view](./functions.html) in allocation mode to find the function doing the allocating.

**What do the thread states Running, Sleeping and Blocked mean?**

`Running` is on-CPU, `Sleeping` is parked or waiting (normal for idle workers), and `Blocked` is an uninterruptible wait, typically disk I/O. Occasional `Blocked` is fine; a thread stuck there is a synchronous I/O bottleneck. Status is a live value, so watch it in the TUI or the JSON API - the final report table shows only the aggregate columns.

**How do I monitor per-thread memory allocation in a Tokio app?**

Enable `hotpath-alloc` and name your worker threads so they're distinguishable in the table. Watch for workers with a low Avg% that keep accumulating Diff - they allocated memory during work that was never released. Pair this with [Tokio runtime monitoring](./tokio_runtime.html) to see worker scheduling stats alongside memory. Many [open-source projects using hotpath](./adoption.html) run exactly this setup.

---

Related: [CPU profiling](./cpu_profiling.html) · [Functions](./functions.html) · [Tokio Runtime](./tokio_runtime.html) · [Configuration](./configuration.html)
