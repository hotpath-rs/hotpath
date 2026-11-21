# Rust function profiling: wall-clock timing and percentiles

hotpath-rs instruments individual Rust functions and code blocks to measure how long they take: call counts, average and percentile durations, and each function's share of the total execution time. It works the same way for sync and async functions. To start profiling functions you'll only need `#[hotpath::main]` and `#[hotpath::measure]` macros:

```rust
#[hotpath::measure]
fn sync_function(sleep: u64) {
    std::thread::sleep(Duration::from_nanos(sleep));
}

#[hotpath::measure]
async fn async_function(sleep: u64) {
    tokio::time::sleep(Duration::from_nanos(sleep)).await;
}

// When using with tokio, place the #[tokio::main] first
#[tokio::main]
#[hotpath::main]
async fn main() {
    for i in 0..100 {
        // Measured functions will automatically send metrics
        sync_function(i);
        async_function(i * 2).await;

        // Measure code blocks with static labels
        hotpath::measure_block!("custom_block", {
            std::thread::sleep(Duration::from_nanos(i * 3))
        });
    }
}
```

When the `hotpath` feature is disabled, all macros are noop and have zero compile or runtime overhead.

Run your program with a hotpath feature:

```bash
cargo run --features=hotpath
```

Output:

```text
[hotpath] 123.86ms | timing, threads

timing - Execution duration of functions.
+-----------------------+-------+-----------+-----------+-----------+---------+
| Function              | Calls | Avg       | P95       | Total     | % Total |
+-----------------------+-------+-----------+-----------+-----------+---------+
| basic::main           | 1     | 123.17 ms | 123.21 ms | 123.17 ms | 100.00% |
+-----------------------+-------+-----------+-----------+-----------+---------+
| basic::async_function | 100   | 1.16 ms   | 1.20 ms   | 116.21 ms | 94.35%  |
+-----------------------+-------+-----------+-----------+-----------+---------+
| basic::sync_function  | 100   | 23.55 µs  | 42.11 µs  | 2.36 ms   | 1.91%   |
+-----------------------+-------+-----------+-----------+-----------+---------+
| custom_block          | 100   | 23.52 µs  | 36.51 µs  | 2.35 ms   | 1.91%   |
+-----------------------+-------+-----------+-----------+-----------+---------+
```

The report also includes a [threads](./threads.md) section, omitted here.

## `#[hotpath::main]` macro

Attribute macro that initializes the background measurement processing when applied. Supports parameters:

- `percentiles = [50, 95, 99]` - Custom percentiles to display, sorted and deduplicated, at most 10 (defaults to `[95]`)
- `format = "json"` - Output format `"table"`, `"json"`, `"json-pretty"`, `"none"` (defaults to `table`)
- `limit = 20` - Maximum number of items to display in every report section except debug (`0` = show all)
- `functions_limit = 20` - Maximum number of functions to display, overrides `limit` (default: `15`, `0` = show all). `channels_limit`, `streams_limit`, `futures_limit`, `rw_locks_limit`, `mutexes_limit` and `threads_limit` do the same for their sections, see [Configuring limits](./profiling_modes.md#hotpathmain-vs-hotpathguardbuilder-api)
- `output_path = "report.json"` - Filesystem path for profiling reports. If not set, the report is written to `stdout`. Can be overridden by `HOTPATH_OUTPUT_PATH`; on Unix, set that env var to `/dev/stdout` or `/dev/stderr` to redirect to the standard streams.
- `report = "functions-timing,channels"` - Report sections spec: `all`, `auto`, an exact comma-separated list of section names, or auto with exclusions like `"auto,-threads"`. Defaults to auto - function and thread sections plus every instrumented section with data (overridden by `HOTPATH_REPORT` env var)

## `#[hotpath::measure]` macro

An attribute macro that instruments functions to send timing/memory measurements to the background processor. Parameters:

- `log = true` - logs the result value when the function returns (requires `std::fmt::Debug` on return type)
- `label = "name"` - replaces the full reported identifier (instead of `module_path::<fn_name>`). Labels must be unique within a crate (shared with `measure_block!` literal labels); a repeated one fails `cargo build`/`test`/`run` with ``symbol `__hotpath_unique_function_name_<crate hash>` is already defined`` (`cargo check` does not report it).
- `impl_type = "Type"` - inserts the enclosing type segment so the registered name becomes `module_path::<Type>::<fn_name>`. Use this for bare `#[hotpath::measure]` on a method inside an `impl` not covered by `measure_all`. Required for correct CPU sampling attribution under `hotpath-cpu` (see [CPU profiling](./cpu_profiling.md)), since the demangled symbol contains the type segment.
- `future = true` - additionally tracks the async function as a future: poll counts, poll durations, and pending/ready/cancelled state transitions. Only valid on `async fn`.

Example:

```rust
#[hotpath::measure(log = true)]
fn compute() -> i32 {
    // The result value will be logged in TUI console
    42
}

#[hotpath::measure(label = "db_query")]
fn fetch_user(id: u64) { /* ... */ }

struct Worker;
impl Worker {
    #[hotpath::measure(impl_type = "Worker")]
    fn run(&self) { /* ... */ }
}
```

<img loading="lazy" src="{{#asset-hash images/functions-log.png}}" alt="hotpath-rs TUI showing function return value logging">

### Async functions and `future = true`

`#[hotpath::measure]` on an `async fn` records its wall-clock time and allocations in the functions report, but nothing about how the future itself behaves. Add `future = true` to also register it in the [futures section](./data_flow.md#futures-monitoring), which shows poll counts, average poll duration, and whether calls completed or were cancelled:

```rust
#[hotpath::measure(future = true)]
async fn fetch_data() -> Vec<u8> {
    // Reported under both functions and futures, keyed by the function path
    vec![1, 2, 3]
}
```

This is the recommended way to monitor async functions: one attribute covers timing, allocations and future lifecycle. The `future!` and `#[future_fn]` macros from the [async data flow](./data_flow.md#futures-monitoring) page are for ad-hoc future expressions.

## `#[hotpath::measure_all]` macro

An attribute macro that applies `#[measure]` to all functions in a `mod` or `impl` block. Useful for bulk instrumentation without annotating each function individually. Can be used on:

- **Inline module declarations** - Instruments all functions within the module
- **Impl blocks** - Instruments all methods in the implementation

Example:

```rust
// Measure all methods in an impl block
#[hotpath::measure_all]
impl Calculator {
    fn add(&self, a: u64, b: u64) -> u64 { a + b }
    fn multiply(&self, a: u64, b: u64) -> u64 { a * b }
    async fn async_compute(&self) -> u64 { /* ... */ }
}

// Measure all functions in a module
#[hotpath::measure_all]
mod math_operations {
    pub fn complex_calculation(x: f64) -> f64 { /* ... */ }
    pub async fn fetch_data() -> Vec<u8> { /* ... */ }
}
```

> **Note:** Once Rust stabilizes [`#![feature(proc_macro_hygiene)]`](https://doc.rust-lang.org/beta/unstable-book/language-features/proc-macro-hygiene.html?highlight=proc_macro_hygiene#proc_macro_hygiene) and [`#![feature(custom_inner_attributes)]`](https://doc.rust-lang.org/beta/unstable-book/language-features/custom-inner-attributes.html), it will be possible to use `#![measure_all]` as an inner attribute directly inside module files (e.g., at the top of `math_operations.rs`) to automatically instrument all functions in that module.

> **Note (CPU sampling):** On inherent impl blocks (`impl Type { ... }`), `measure_all` auto-injects the type segment so methods are registered as `module_path::<Type>::<method>` - this matches the demangled symbol used by `hotpath-cpu` attribution. Trait impls (`impl Trait for Type`) are still instrumented for timing/allocation, but their demangled symbols use the `<Type as Trait>::method` form, so CPU sampling will not attribute samples to those methods.

## `#[hotpath::skip]` macro

A marker attribute that excludes specific functions from instrumentation when used within a module or impl block annotated with `#[measure_all]`. The function executes normally but doesn't send measurements to the profiling system.

Example:

```rust
#[hotpath::measure_all]
mod operations {
    pub fn important_function() { /* ... */ } // Measured

    #[hotpath::skip]
    pub fn not_so_important_function() { /* ... */ } // NOT measured
}
```

## `hotpath::measure_block!` macro

Macro that measures the execution time of a code block with a static string label. A literal label must be unique within the crate, across `measure_block!` and `#[measure(label = ...)]` sites alike; a repeated one fails the build. Labels passed as runtime expressions are not checked.

```rust
#[hotpath::main]
fn main() {
    for i in 0..100 {
        // Measure code blocks with static labels
        hotpath::measure_block!("custom_block", {
            std::thread::sleep(Duration::from_nanos(i * 3))
        });
    }
}
```

If `hotpath` feature is disabled, the code inside block will still execute.

## Function sink

`hotpath::set_function_sink` installs a callback that sees every `#[measure]` and `measure_block!` scope as it runs, instead of only the aggregated report. Use it to feed individual calls into an external recorder, such as a timeline viewer or a tracing bridge.

The sink is behind its own feature, so builds that do not use it carry no sink state in their measurement guards:

```toml
[features]
hotpath-sink = ["hotpath", "hotpath/hotpath-sink"]
```

```rust
use std::sync::atomic::{AtomicU64, Ordering};

use hotpath::{FunctionSink, ScopeKind};

#[derive(Default)]
struct Timeline {
    next: AtomicU64,
}

impl FunctionSink for Timeline {
    fn enter(&self, name: &'static str, kind: ScopeKind) -> u64 {
        // record the start of `name`
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    fn exit(&self, name: &'static str, kind: ScopeKind, token: u64) {
        // record the end of the scope started by `token`
    }
}

#[hotpath::main]
fn main() {
    hotpath::set_function_sink(Timeline::default()).unwrap();
    // ...
}
```

- Both methods run inline on the instrumented thread, so keep them cheap and non-blocking.
- `ScopeKind::Sync` scopes are sync function bodies: `exit` runs on the thread that called `enter`, and they nest strictly per thread. `ScopeKind::Async` scopes (`async fn`, `measure_block!`, the `#[main]` wrapper) can suspend across `.await` and exit on another thread. Pair those by the token `enter` returned.
- The sink is called whether or not a guard is alive and regardless of time sampling. Scopes filtered out by `HOTPATH_FOCUS` are not reported, and neither are scopes entered before the sink was installed.
- Only one sink can be installed per process. A second call returns `FunctionSinkAlreadySet`.
- Without the `hotpath-sink` feature, `set_function_sink` is a no-op that drops the sink, so the call site needs no feature gate.
- With `hotpath-sink` enabled but no sink installed, each scope pays for one extra atomic load.

## Memory and allocations profiling

<a id="memory-profiling-modes"></a><a id="custom-inner-allocator"></a><a id="allocation-tracking-with-hotpathguardbuilder"></a>In addition to execution timing, hotpath-rs tracks heap allocations made by instrumented Rust functions. Enable the `hotpath-alloc` feature to add an `alloc-bytes` section to the report, identify allocation hotspots and compare allocation statistics:

```bash
cargo run --features='hotpath-alloc'
```

See the [Rust memory profiling guide](memory_profiling.md) for per-function allocation metrics, exclusive vs cumulative tracking, per-thread memory usage, RSS monitoring, and custom allocator configuration.
