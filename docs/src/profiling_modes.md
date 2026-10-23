# Profiling modes: static reports and live monitoring

`hotpath` supports two complementary approaches to Rust performance profiling and runtime monitoring. Not sure which profiling technique fits your bottleneck? The [complete guide to profiling Rust](/blog/profiling-rust-guide#which-rust-profiling-technique-do-you-need) walks through each one on a real example.

## Static reports

Best for CLI tools, tests, or short-lived applications. On exit, `hotpath` prints a summary of execution time, memory usage, and timing percentiles. Reports can be rendered as readable tables or exported as JSON for automated analysis.

Every instrumented program prints a performance report automatically when executed with the `hotpath` feature enabled.

```bash
cargo run --features=hotpath
```

<img loading="lazy" src="{{#asset-hash images/hotpath-timing-report.png}}" alt="hotpath-rs timing profiling report showing per-function execution statistics">

Use `--features='hotpath-alloc'` to print memory usage report:

```bash
cargo run --features='hotpath-alloc'
```

<img loading="lazy" src="{{#asset-hash images/hotpath-alloc-report.png}}" alt="hotpath-rs memory allocation profiling report showing per-function byte counts">

By default, allocation reports sort and display by bytes. Set `HOTPATH_ALLOC_METRIC=count` to use allocation count as the primary metric instead:

```bash
HOTPATH_ALLOC_METRIC=count cargo run --features='hotpath-alloc'
```

### `hotpath::main` vs `HotpathGuardBuilder` API

`#[hotpath::main]` is the simplest way to enable profiling. It creates a [`HotpathGuard`](https://docs.rs/hotpath/latest/hotpath/struct.HotpathGuard.html) for the entry-point function, and the profiling report is generated automatically when this function returns and that guard is dropped. You can use `hotpath::main` macro on any function not only `main`.

[`HotpathGuardBuilder`](https://docs.rs/hotpath/latest/hotpath/struct.HotpathGuardBuilder.html) gives you manual control over the profiling guard lifetime. The report is generated when the returned `HotpathGuard` is dropped. You can start profiling later, stop it earlier, or execute a custom logic using `before_shutdown` immediately before report generation.

- `#[hotpath::main]` is best when you want profiling to cover the whole program entry point with minimal setup.
- `HotpathGuardBuilder` is best when you want to profile only part of a program, or control exactly when the report is generated.
- Only one `HotpathGuard` may be alive at a time. Creating a second guard, whether from `#[hotpath::main]` or `HotpathGuardBuilder`, will panic.

**Configuring limits**

`limit` sets a global cap on items shown in every report section except debug, which is never truncated. Per-resource limits override the global value for that section:

```rust
// Global limit applies to all sections
#[hotpath::main(limit = 10)]
fn main() { /* ... */ }

// Per-resource limits override the global limit
#[hotpath::main(limit = 10, functions_limit = 20, channels_limit = 5)]
fn main() { /* ... */ }

// Set limit = 0 to display all instrumented resources without truncation
#[hotpath::main(limit = 0)]
fn main() { /* ... */ }
```

The same applies to the builder API - `limit` sets all sections, then individual setters override:

```rust
use std::time::Duration;

#[hotpath::measure]
fn example_function() {
    std::thread::sleep(Duration::from_millis(10));
}

fn main() {
    let guard = hotpath::HotpathGuardBuilder::new("my_program")
        .percentiles(&[95.0, 99.0])
        .limit(10)
        .functions_limit(20)
        .format(hotpath::Format::Table)
        .build();

    example_function();

    // Dropping the guard shuts down profiling and writes the report.
    drop(guard);

    // This exits immediately, so #[hotpath::main] would not generate a report.
    std::process::exit(1);
}
```

The macro accepts `limit`, `functions_limit`, `channels_limit`, `streams_limit`, `futures_limit`, `rw_locks_limit`, `mutexes_limit` and `threads_limit`. The builder additionally has `sql_limit`, `http_limit`, `server_limit` and `io_limit`; each of these sections also has a `HOTPATH_*_LIMIT` [env var](configuration.md#limits). The debug section has no limit.

### Configuring static reports

| Variable | Description |
|----------|-------------|
| `HOTPATH_OUTPUT_FORMAT` | Output format: `table` (default), `json`, `json-pretty`, or `none`. Using `none` silences output while keeping the metrics server and MCP server active. |
| `HOTPATH_OUTPUT_PATH` | Filesystem path for profiling reports. If unset, reports are written to `stdout`. When set, this env var takes precedence over programmatic `output_path` config. On Unix, use `/dev/stdout` or `/dev/stderr` to redirect to the standard streams. |
| `HOTPATH_REPORT` | Report sections spec: `all`, `auto`, an exact comma-separated list of section names, or auto with exclusions like `auto,-threads`. Defaults to `auto` - function and thread sections plus every instrumented section with data. |
| `HOTPATH_FOCUS` | Filter profiled functions by name. Plain text does substring matching; wrap in `/pattern/` for regex (e.g. `HOTPATH_FOCUS="/^(compute\|process)/"`). |
| `HOTPATH_METRICS_SERVER_OFF` | Set to `true` or `1` to disable the HTTP metrics server. Useful when you only need a static report and don't want to use a TUI. |

Example - write a JSON report containing only function timing and thread usage metrics to a file:

```bash
HOTPATH_OUTPUT_FORMAT=json \
HOTPATH_OUTPUT_PATH=report.json \
HOTPATH_REPORT=functions-timing,threads \
cargo run --features=hotpath
```

The builder API selects report sections with `.report("functions-timing,threads")`, which takes the same spec as `HOTPATH_REPORT`, or with typed lists: `.sections(vec![hotpath::Section::FunctionsTiming])` shows exactly the listed sections, and `.sections_exclude(vec![hotpath::Section::Threads])` hides sections from the auto report. The last of the three calls wins, and `HOTPATH_REPORT` overrides them all.

### Timed shutdown

`HOTPATH_SHUTDOWN_MS` forces the program to exit and print the report after a fixed duration. This is useful for profiling long-running processes (HTTP servers, workers) where you want to collect metrics for a predefined period without manual intervention. It also enables deterministic benchmarks - run the same workload for a fixed window across different git commits and compare the reports.

```bash
HOTPATH_SHUTDOWN_MS=10000 \
HOTPATH_OUTPUT_FORMAT=json \
HOTPATH_OUTPUT_PATH=tmp/report.json \
cargo run --features=hotpath
```

Use `before_shutdown` in the `HotpathGuardBuilder` API to run cleanup logic (flush connections, drain queues) before the report is generated.

With the builder API, `build_with_shutdown(duration)` replaces `build()` for programs that never return from their entry point: it moves the guard to a background thread, and after `duration` (or `HOTPATH_SHUTDOWN_MS`, which takes precedence) prints the report and exits the process. A zero duration with no env var keeps the guard alive until the process exits.

## Live TUI dashboard

Best for long-running processes like HTTP servers, or background workers. It continuously displays function performance metrics, allocation counters, and channel/stream throughput while the application is running. This mode helps diagnose runtime bottlenecks, queue buildup, and data flow issues that are not visible in static summaries.

Install the TUI with:

```
cargo install hotpath --features=tui
```

Run the dashboard:

```
hotpath console
```

Then launch your instrumented application (with `hotpath` feature enabled) in a separate terminal to see live performance metrics.

<video loading="lazy" width="100%" loop muted playsinline controls poster="{{#asset-hash images/hotpath-live-dashboard-poster.jpg}}">
  <source src="{{#asset-hash videos/hotpath-live-dashboard.mp4}}" type="video/mp4">
</video>

## Optional vs non-optional

`hotpath` can be added either as a regular dependency or as an `optional` one. Both work; they differ only in how much `cfg` gating the code needs.

### Non-optional dependency (recommended)

It is safe to keep `hotpath` as a regular, non-optional dependency. Unless the `hotpath` feature is enabled:

- **Zero compile-time overhead** - the crate contains no profiling code and pulls in no dependencies. All of its dependencies are optional and are only compiled when the feature is on.
- **Zero runtime overhead** - `#[hotpath::main]`, `#[hotpath::measure]`, `measure_block!`, and all channel, lock, stream and future wrappers expand to a no-op that returns the original code unchanged.

This is the recommended setup from the [installation guide](introduction.md#manual-installation):

```toml
[dependencies]
hotpath = "{{HOTPATH_VERSION}}"

[features]
hotpath = ["hotpath/hotpath"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
```

With this config, macros can be used directly in the code without any `cfg` gating, and builds without `--features=hotpath` are identical to builds that don't depend on `hotpath` at all.

### Optional dependency

If you prefer not to have `hotpath` in the dependency tree at all unless profiling is enabled, declare it as `optional` and enable it from your feature:

```toml
[dependencies]
hotpath = { version = "{{HOTPATH_VERSION}}", optional = true }

[features]
hotpath = ["dep:hotpath", "hotpath/hotpath"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
```

Because the crate is now absent from non-profiling builds, every use of its macros must be gated with `cfg_attr` / `cfg`:

```rust
#[cfg_attr(feature = "hotpath", hotpath::measure)]
fn sync_function() {
    // ...
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
async fn async_function() {
    // ...
}

#[tokio::main]
#[cfg_attr(feature = "hotpath", hotpath::main)]
async fn main() {
    sync_function();
    async_function().await;
}
```

Function-like macros and wrappers need a `cfg` branch for each variant:

```rust
#[cfg(feature = "hotpath")]
let (tx, mut rx) = hotpath::channel!(tokio::sync::mpsc::channel::<String>(100));
#[cfg(not(feature = "hotpath"))]
let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(100);
```

Both setups are run the same way:

```bash
cargo run --features=hotpath
```

The optional variant trades extra `cfg` noise in the code for an empty dependency entry when profiling is off. Since the non-optional setup already has no compile-time or runtime cost, prefer it unless your project policy requires the dependency to be absent.
