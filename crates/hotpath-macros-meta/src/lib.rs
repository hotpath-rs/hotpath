use proc_macro::TokenStream;

#[cfg(feature = "hotpath-meta")]
mod lib_on;

#[cfg(not(feature = "hotpath-meta"))]
mod lib_off;

/// Splits `__callgrind_block!` input at its first top-level comma into the
/// label and the body, untouched. A `$label:literal` holds no comma.
fn split_block_input(input: TokenStream) -> (TokenStream, TokenStream) {
    let mut tokens = input.into_iter();
    let label = tokens
        .by_ref()
        .take_while(|t| !matches!(t, proc_macro::TokenTree::Punct(p) if p.as_char() == ','))
        .collect();
    (label, tokens.collect())
}

/// Initializes the hotpath profiling system and generates a performance report on program exit.
///
/// Apply to `main` (or another entry point) to enable profiling: starts the background
/// workers and prints the report when the program exits. The annotated function itself is
/// also measured.
///
/// For programmatic control over the same options, see
/// [`HotpathGuardBuilder`](../hotpath_meta/struct.HotpathGuardBuilder.html).
///
/// # Parameters
///
/// * `percentiles` - Array of percentile values (0.0-100.0) to compute, e.g. `[50, 95, 99.9]`. Sorted and deduplicated, at most 10. Default: `[95]`
/// * `format` - Output format: `"table"` (default), `"json"`, `"json-pretty"`, or `"none"`
/// * `limit` - Global maximum number of items shown in each report section (functions, channels, streams, futures, threads). `0` = unlimited.
/// * `functions_limit` - Maximum number of functions shown in the report. Overrides `limit` for functions.
/// * `channels_limit` - Maximum number of channels shown in the report. Overrides `limit` for channels.
/// * `streams_limit` - Maximum number of streams shown in the report. Overrides `limit` for streams.
/// * `futures_limit` - Maximum number of futures shown in the report. Overrides `limit` for futures.
/// * `threads_limit` - Maximum number of threads shown in the report. Overrides `limit` for threads.
/// * `output_path` - File path for the report. Defaults to stdout. Overridden by `HOTPATH_META_OUTPUT_PATH` env var.
/// * `report` - Report sections spec: `"all"`, `"auto"`, an exact list like `"functions-timing,channels"`, or auto with exclusions like `"auto,-threads"`. Default: auto (function and thread sections plus every instrumented section with data). Overridden by `HOTPATH_META_REPORT` env var.
/// * `allocator` - Optional allocator used when `hotpath-alloc-meta` is enabled. The path must
///   name a unit struct implementing `GlobalAlloc` (e.g. `mimalloc::MiMalloc`); it is used
///   both as the type and the value of the inner allocator. Defaults to `std::alloc::System`.
///
/// Environment variable precedence for report output:
/// `HOTPATH_META_LIMIT`, `HOTPATH_META_FUNCTIONS_LIMIT`,
/// `HOTPATH_META_CHANNELS_LIMIT`, `HOTPATH_META_STREAMS_LIMIT`,
/// `HOTPATH_META_FUTURES_LIMIT`, and `HOTPATH_META_THREADS_LIMIT`
/// override the matching macro arguments. Per-resource env vars override
/// `HOTPATH_META_LIMIT`.
///
/// # Examples
///
/// Basic usage with default settings (P95 percentile, table format):
///
/// ```rust,no_run
/// #[hotpath_meta::main]
/// fn main() {
///     // Your code here
/// }
/// ```
///
/// Custom percentiles:
///
/// ```rust,no_run
/// #[tokio::main]
/// #[hotpath_meta::main(percentiles = [50, 90, 95, 99.9])]
/// async fn main() {
///     // Your code here
/// }
/// ```
///
/// JSON output to file:
///
/// ```rust,no_run
/// #[hotpath_meta::main(format = "json-pretty", output_path = "report.json")]
/// fn main() {
///     // Your code here
/// }
/// ```
///
/// Select exact report sections, or hide a section from the auto report:
///
/// ```rust,no_run
/// #[hotpath_meta::main(report = "functions-timing,channels")]
/// fn main() {
///     // Your code here
/// }
/// ```
///
/// ```rust,no_run
/// #[hotpath_meta::main(report = "auto,-threads")]
/// fn main() {
///     // Your code here
/// }
/// ```
///
/// Per-resource limits:
///
/// ```rust,no_run
/// #[hotpath_meta::main(limit = 10, functions_limit = 20, channels_limit = 5)]
/// fn main() {
///     // Your code here
/// }
/// ```
///
/// # Usage with Tokio
///
/// When using with tokio, place `#[tokio::main]` before `#[hotpath_meta::main]`:
///
/// ```rust,no_run
/// #[tokio::main]
/// #[hotpath_meta::main]
/// async fn main() {
///     // Your code here
/// }
/// ```
///
/// # Limitations
///
/// Only one hotpath guard can be active at a time. Creating a second guard (either via this
/// macro or via [`HotpathGuardBuilder`](../hotpath_meta/struct.HotpathGuardBuilder.html)) will cause a panic.
///
/// # See Also
///
/// * [`measure`](macro@measure) - Attribute macro for instrumenting functions
/// * [`measure_block!`](../hotpath_meta/macro.measure_block.html) - Macro for measuring code blocks
/// * [`HotpathGuardBuilder`](../hotpath_meta/struct.HotpathGuardBuilder.html) - Programmatic alternative to this macro
#[proc_macro_attribute]
pub fn main(attr: TokenStream, item: TokenStream) -> TokenStream {
    #[cfg(feature = "hotpath-meta")]
    {
        lib_on::main_impl(attr, item)
    }
    #[cfg(not(feature = "hotpath-meta"))]
    {
        lib_off::main_impl(attr, item)
    }
}

/// Instruments a function to measure execution time or memory allocations.
///
/// Automatically detects sync vs async and inserts the appropriate measurement guard.
/// Compiles to zero overhead when the `hotpath-meta` feature is disabled.
///
/// # Measurements
///
/// * **Time profiling** (default) - execution duration via high-precision timers
/// * **Allocation profiling** (`hotpath-alloc-meta` feature) - bytes allocated and allocation count
///
/// # Parameters
///
/// * `log` - If `true`, logs the return value on each call (requires `Debug` on return type)
/// * `future` - If `true`, also tracks the future lifecycle (poll count, state transitions, cancellation). Only valid on async functions.
///
/// # Examples
///
/// ```rust,no_run
/// #[hotpath_meta::measure]
/// fn process(data: &[u8]) -> usize {
///     data.len()
/// }
///
/// #[hotpath_meta::measure(log = true)]
/// fn compute() -> i32 {
///     42
/// }
///
/// #[hotpath_meta::measure(future = true)]
/// async fn fetch_data() -> Vec<u8> {
///     vec![1, 2, 3]
/// }
/// ```
///
/// # Async Allocation Limitation
///
/// Allocation profiling requires `current_thread` tokio runtime because thread-local
/// tracking cannot follow tasks across threads. Time profiling works with any runtime.
///
/// # See Also
///
/// * [`main`](macro@main) - Initializes the profiling system
/// * [`measure_all`](macro@measure_all) - Bulk instrumentation for modules and impl blocks
/// * [`measure_block!`](../hotpath_meta/macro.measure_block.html) - Instruments code blocks
#[proc_macro_attribute]
pub fn measure(attr: TokenStream, item: TokenStream) -> TokenStream {
    #[cfg(feature = "hotpath-meta")]
    {
        lib_on::measure_impl(attr, item)
    }
    #[cfg(not(feature = "hotpath-meta"))]
    {
        lib_off::measure_impl(attr, item)
    }
}

/// Instruments an async function to track its lifecycle as a Future.
///
/// Wraps the function body with the `future!` macro to track poll counts,
/// state transitions (pending/ready/cancelled), and optionally the output value.
/// Can only be applied to `async fn`.
///
/// # Parameters
///
/// * `log` - If `true`, logs the output value on completion (requires `Debug` on return type)
///
/// # Examples
///
/// ```rust,no_run
/// #[hotpath_meta::future_fn]
/// async fn fetch_data() -> Vec<u8> {
///     vec![1, 2, 3]
/// }
///
/// #[hotpath_meta::future_fn(log = true)]
/// async fn compute() -> i32 {
///     42
/// }
/// ```
///
/// # See Also
///
/// * [`measure`](macro@measure) - Instruments execution time / allocations
/// * [`future!`](../hotpath_meta/macro.future.html) - Declarative macro for wrapping future expressions
#[proc_macro_attribute]
pub fn future_fn(attr: TokenStream, item: TokenStream) -> TokenStream {
    #[cfg(feature = "hotpath-meta")]
    {
        lib_on::future_fn_impl(attr, item)
    }
    #[cfg(not(feature = "hotpath-meta"))]
    {
        lib_off::future_fn_impl(attr, item)
    }
}

/// Marks a function to be excluded from profiling when used with [`measure_all`](macro@measure_all).
///
/// # Usage
///
/// ```rust,no_run
/// #[hotpath_meta::measure_all]
/// impl MyStruct {
///     fn important_method(&self) {
///         // This will be measured
///     }
///
///     #[hotpath_meta::skip]
///     fn not_so_important_method(&self) -> usize {
///         // This will NOT be measured
///         self.value
///     }
/// }
/// ```
///
/// # See Also
///
/// * [`measure_all`](macro@measure_all) - Bulk instrumentation macro
/// * [`measure`](macro@measure) - Individual function instrumentation
#[proc_macro_attribute]
pub fn skip(attr: TokenStream, item: TokenStream) -> TokenStream {
    #[cfg(feature = "hotpath-meta")]
    {
        lib_on::skip_impl(attr, item)
    }
    #[cfg(not(feature = "hotpath-meta"))]
    {
        lib_off::skip_impl(attr, item)
    }
}

/// Instruments all functions in a module or impl block with [`measure`](macro@measure).
///
/// # Usage
///
/// On modules:
///
/// ```rust,no_run
/// #[hotpath_meta::measure_all]
/// mod my_module {
///     fn function_one() {
///         // This will be automatically measured
///     }
///
///     fn function_two() {
///         // This will also be automatically measured
///     }
/// }
/// ```
///
/// On impl blocks:
///
/// ```rust,no_run
/// struct MyStruct;
///
/// #[hotpath_meta::measure_all]
/// impl MyStruct {
///     fn method_one(&self) {
///         // This will be automatically measured
///     }
///
///     fn method_two(&self) {
///         // This will also be automatically measured
///     }
/// }
/// ```
///
/// # See Also
///
/// * [`measure`](macro@measure) - Attribute macro for instrumenting individual functions
/// * [`main`](macro@main) - Attribute macro that initializes profiling
/// * [`skip`](macro@skip) - Marker to exclude specific functions from measurement
#[proc_macro_attribute]
pub fn measure_all(attr: TokenStream, item: TokenStream) -> TokenStream {
    #[cfg(feature = "hotpath-meta")]
    {
        lib_on::measure_all_impl(attr, item)
    }
    #[cfg(not(feature = "hotpath-meta"))]
    {
        lib_off::measure_all_impl(attr, item)
    }
}

/// Fails the build when two call sites of one kind share a literal label within
/// a crate, with ``symbol `__hotpath_meta_unique_<kind>_<label>_<hash>` is
/// already defined``. Only codegen reports it, not `cargo check` or clippy.
#[doc(hidden)]
#[proc_macro]
pub fn __unique_label(input: TokenStream) -> TokenStream {
    #[cfg(feature = "hotpath-meta")]
    {
        lib_on::unique_label_impl(input)
    }
    #[cfg(not(feature = "hotpath-meta"))]
    {
        lib_off::unique_label_impl(input)
    }
}

/// Wraps a `measure_block!` body (`"label", expr`) in a Callgrind marker
/// under `hotpath-callgrind-meta`, unless running it as a closure would change
/// what it does; expands to the body unchanged otherwise.
#[doc(hidden)]
#[proc_macro]
pub fn __callgrind_block(input: TokenStream) -> TokenStream {
    #[cfg(feature = "hotpath-meta")]
    {
        lib_on::callgrind_block_impl(input)
    }
    #[cfg(not(feature = "hotpath-meta"))]
    {
        lib_off::callgrind_block_impl(input)
    }
}
