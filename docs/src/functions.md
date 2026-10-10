# Function performance profiling: timing and memory metrics

To start profiling functions performance you'll only need `#[hotpath::main]` and `#[hotpath::measure]` macros:

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

## Exclusive time mode

By default, function timing is **inclusive**: a function's duration covers everything that happens between its entry and exit, including nested measured functions. A parent therefore always reports at least as much time as its children, and a recursive function counts its nested levels again on every level.

Set `HOTPATH_TIME_EXCLUSIVE=true` to switch to **exclusive** (self) time, where each function reports its own time without the time spent in the measured functions it calls. Time spent in functions that are not measured (or are filtered out by `HOTPATH_FOCUS`) stays with the nearest measured caller. In this mode the `% Total` column is relative to the sum of all rows, so the rows add up to 100%, and recursive functions are counted correctly.

Things to keep in mind:

- **Async functions report busy time.** The exclusive time of an async function is summed over the polls of its body, so time suspended at an `.await` (timers, I/O, waiting for another task) is not counted. A sync function's exclusive time still includes blocking calls like `std::thread::sleep`.
- **Function time sampling is disabled.** A call that is not timed has no duration to subtract from its caller, so `HOTPATH_TIME_SAMPLING_RATE` and `HOTPATH_FUNCTIONS_TIME_SAMPLING_RATE` are ignored for functions (a warning is printed). Other resource types keep their sampling rates.
- **Guards dropped on another thread report no duration.** This only affects `measure_block!` blocks that span an `.await` in a task that migrates between threads. Such blocks are also approximate on a single thread, because measured polls of other tasks interleaved on that thread are subtracted from them.
- `future!` wrappers are not functions: time spent polling them stays in the enclosing function's exclusive time.

## Memory and allocations profiling

In addition to time-based profiling, `hotpath` can track memory allocations. This feature uses a custom global allocator from [allocation-counter crate](https://github.com/fornwall/allocation-counter) to intercept all memory allocations and provides detailed statistics about memory usage per function.

Run your program with the allocation tracking feature to print a similar report:

```
cargo run --features='hotpath,hotpath-alloc'
```

<img loading="lazy" src="{{#asset-hash images/hotpath-alloc-report.png}}" alt="hotpath-rs memory allocation profiling report showing per-function byte counts">

## Memory profiling modes

By default, allocation tracking is **exclusive**, meaning each function only reports allocations made directly at its own level, excluding nested instrumented calls.

To switch to **cumulative** mode (where a function's allocation count includes all allocations from nested instrumented calls), set `HOTPATH_ALLOC_CUMULATIVE=true`. Note that cumulative mode produces invalid results for recursive functions because the same allocations are counted multiple times as they propagate up through each recursive frame.

## Custom inner allocator

The tracking allocator forwards every allocation to an inner allocator, `std::alloc::System` by default. To profile a program that uses a different allocator, pass it via the `allocator` parameter of `#[hotpath::main]`. For example, with [tikv-jemallocator](https://github.com/tikv/jemallocator):

```toml
[dependencies]
tikv-jemallocator = "0.6"
```

```rust
#[hotpath::measure]
fn alloc_work() {
    let buf = vec![0u8; 4096];
    std::hint::black_box(&buf);
}

#[hotpath::main(allocator = tikv_jemallocator::Jemalloc)]
fn main() {
    for _ in 0..100 {
        alloc_work();
    }
}
```

```bash
cargo run --features='hotpath,hotpath-alloc'
```

The report now shows per-function allocation stats measured through jemalloc: every allocation is counted by the tracking wrapper, then served by jemalloc instead of the system allocator.

The path must name a unit struct implementing `GlobalAlloc` (like `MiMalloc` from [mimalloc](https://github.com/purpleprotocol/mimalloc_rust) or `Jemalloc` from [tikv-jemallocator](https://github.com/tikv/jemallocator)). When the `hotpath-alloc` feature is disabled, the parameter is ignored and no allocator is installed, so combine it with your own `#[global_allocator]` behind a feature gate if the program should also use the allocator in normal builds.

All four `GlobalAlloc` methods (`alloc`, `dealloc`, `alloc_zeroed`, `realloc`) forward to the inner allocator's native implementations, so wrapping does not change its zeroed-allocation or reallocation behavior. A successful `realloc` is tracked as a deallocation of the old size plus an allocation of the new size.

## Allocation tracking with `HotpathGuardBuilder`

The `#[hotpath::main]` macro installs the tracking allocator for you. The [`HotpathGuardBuilder`](https://docs.rs/hotpath/latest/hotpath/struct.HotpathGuardBuilder.html) API does not do that (a `#[global_allocator]` must be declared as a static item), so when initializing hotpath programmatically you must declare it yourself:

```rust
#[global_allocator]
static GLOBAL: hotpath::CountingAllocator = hotpath::CountingAllocator::new();

fn main() {
    let _hotpath = hotpath::HotpathGuardBuilder::new("main").build();
    // ...
}
```

Use `hotpath::CountingAllocator::with(...)` to wrap a custom inner allocator:

```rust
#[global_allocator]
static GLOBAL: hotpath::CountingAllocator<tikv_jemallocator::Jemalloc> =
    hotpath::CountingAllocator::with(tikv_jemallocator::Jemalloc);
```

The declaration needs no feature gating: `CountingAllocator` is exported in every feature configuration, and when `hotpath-alloc` (or `hotpath` itself) is disabled it is a pure pass-through to the inner allocator with no tracking overhead. This also makes it the simplest way to keep a custom allocator active in normal builds: the program always runs on jemalloc, and allocation tracking switches on only with `hotpath-alloc`.

Don't combine a manual declaration with `#[hotpath::main]` - under `hotpath-alloc` the macro installs its own `#[global_allocator]`, and two declarations fail to compile. Use the `allocator` parameter with the macro, and the manual declaration with the builder.
