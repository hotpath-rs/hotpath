#[doc(hidden)]
pub use hotpath_macros::{future_fn, main, measure, measure_all, skip};
use std::sync::OnceLock;

use crate::instant::Instant;

pub(crate) static START_TIME: OnceLock<Instant> = OnceLock::new();

/// Upper bound shared by every duration histogram; longer measurements are
/// clamped to this value.
pub(crate) const MAX_DURATION_NS: u64 = 1_000_000_000_000; // 1000s

#[inline]
pub(crate) fn elapsed_since_start_ns(end: Instant) -> u64 {
    START_TIME
        .get()
        .map(|start| end.duration_since(*start).as_nanos() as u64)
        .unwrap_or(0)
}

#[inline]
pub(crate) fn current_elapsed_ns() -> u64 {
    START_TIME
        .get()
        .map(|start| start.elapsed().as_nanos() as u64)
        .unwrap_or(0)
}

pub(crate) use hotpath_drain as batch;
pub(crate) mod caller_stack;
pub(crate) mod channels;
#[doc(hidden)]
pub mod debug;
#[doc(hidden)]
pub mod futures;
#[doc(hidden)]
pub mod http;
#[doc(hidden)]
pub mod io;
pub(crate) mod locations;
pub(crate) mod mutexes;
pub(crate) mod rw_locks;
pub(crate) mod server;
pub(crate) mod sql;
pub(crate) mod streams;
#[cfg(feature = "threads")]
pub(crate) mod threads;
#[cfg(feature = "tokio")]
#[doc(hidden)]
pub mod tokio_runtime;

#[doc(hidden)]
pub mod functions;

pub use channels::{InstrumentChannelWrap, InstrumentChannelWrapLog};
pub use futures::{InstrumentFuture, InstrumentFutureLog};
pub use http::{InstrumentHttpClient, ReqwestHttpMiddleware, UreqHttpMiddleware};
pub use io::io_unwrap;
pub use mutexes::InstrumentMutex;
pub use rw_locks::InstrumentRwLock;
pub use server::AxumLayer;
#[cfg(feature = "diesel")]
pub use sql::instrument_diesel_sql;
#[cfg(feature = "sqlx")]
pub use sql::sqlx_tracing_layer;
#[cfg(feature = "toasty")]
pub use sql::toasty_tracing_layer;
pub use streams::{InstrumentStream, InstrumentStreamLog};

#[cfg(feature = "hotpath-cloud")]
pub(crate) mod ci_info;
#[cfg(feature = "hotpath-cloud")]
pub(crate) mod cloud;
#[cfg(feature = "hotpath-cloud")]
pub(crate) mod git_info;
pub(crate) mod hotpath_guard;
pub(crate) mod merge;
#[cfg(feature = "hotpath-prometheus")]
pub(crate) mod native_histograms;
pub(crate) mod report;
pub(crate) mod report_meta;
pub(crate) mod sampling;

#[doc(hidden)]
pub use locations::{register_location, Location};

cfg_if::cfg_if! {
    if #[cfg(feature = "hotpath-meta")] {
        /// Profiler-internal shared-state lock: the meta-instrumented
        /// `std::sync::RwLock` wrapper when self-profiling is enabled, the
        /// plain std lock otherwise. Construct via [`meta_rw_lock!`].
        pub(crate) type MetaRwLock<T> = hotpath_meta::wrap::std::sync::RwLock<T>;

        /// A macro rather than a function so each lock registers its own
        /// call-site source; a shared source would make meta's label
        /// resolution append dedup suffixes (`sql_state-2`).
        macro_rules! meta_rw_lock {
            ($label:expr, $value:expr $(,)?) => {
                hotpath_meta::rw_lock!(std::sync::RwLock::new($value), label = $label)
            };
        }

        /// A profiler-internal block measured by the self-profiling copy.
        #[cfg(feature = "hotpath-alloc")]
        macro_rules! meta_measure_block {
            ($label:literal, $expr:expr $(,)?) => {
                hotpath_meta::measure_block!($label, $expr)
            };
        }
    } else {
        pub(crate) type MetaRwLock<T> = std::sync::RwLock<T>;

        macro_rules! meta_rw_lock {
            ($label:expr, $value:expr $(,)?) => {{
                let _ = $label;
                std::sync::RwLock::new($value)
            }};
        }

        #[cfg(feature = "hotpath-alloc")]
        macro_rules! meta_measure_block {
            ($label:literal, $expr:expr $(,)?) => {
                $expr
            };
        }
    }
}

#[cfg(feature = "hotpath-alloc")]
pub(crate) use meta_measure_block;
pub(crate) use meta_rw_lock;

pub use functions::allocator::CountingAllocator;
pub use functions::{
    measure_async, measure_async_future, measure_async_future_log, measure_async_log,
    measure_sync_log, MeasurementGuardAsync, MeasurementGuardSync,
};
pub use hotpath_guard::{HotpathGuard, HotpathGuardBuilder};

#[must_use = "guard is dropped immediately without suspending tracking"]
pub(crate) struct SuspendAllocTracking {
    #[cfg(feature = "hotpath-alloc")]
    previous_enabled: bool,
}

impl SuspendAllocTracking {
    #[inline]
    pub(crate) fn new() -> Self {
        #[cfg(feature = "hotpath-alloc")]
        {
            let previous_enabled = functions::alloc::core::suspend_alloc_tracking();
            Self { previous_enabled }
        }
        #[cfg(not(feature = "hotpath-alloc"))]
        {
            Self {}
        }
    }
}

impl Drop for SuspendAllocTracking {
    #[inline]
    fn drop(&mut self) {
        #[cfg(feature = "hotpath-alloc")]
        functions::alloc::core::resume_alloc_tracking(self.previous_enabled);
    }
}

/// Measures the execution time or memory allocations of a code block.
///
/// This macro wraps a block of code with profiling instrumentation, similar to the
/// [`measure`](hotpath_macros::measure) attribute macro but for inline code blocks.
/// The block is labeled with a static string identifier.
///
/// # Arguments
///
/// * `$label` - A static string label to identify this code block in the profiling report
/// * `$expr` - The expression or code block to measure
///
/// # Behavior
///
/// The macro automatically uses the appropriate measurement based on enabled feature flags:
/// - **Time profiling** (default): Measures execution duration
/// - **Allocation profiling**: Tracks memory allocations when allocation features are enabled
/// - **Instruction counts** (`hotpath-callgrind`): Counted for a literal label, unless the
///   block contains `?`, `return`, `.await`, a `break`/`continue` leaving it, or a macro
///   other than the standard formatting and assert ones
///
/// # Examples
///
/// ```rust
/// # {
/// use std::time::Duration;
///
/// hotpath::measure_block!("data_processing", {
///     // Your code here
///     std::thread::sleep(Duration::from_millis(10));
/// });
/// # }
/// ```
///
/// # See Also
///
/// * [`measure`](hotpath_macros::measure) - Attribute macro for instrumenting functions
/// * [`main`](hotpath_macros::main) - Attribute macro that initializes profiling
#[macro_export]
macro_rules! measure_block {
    // A literal label shares the functions namespace with `#[measure(label)]`,
    // so both are checked for uniqueness under the `function` kind. The block
    // wrapping turns the literal into a plain expression for the arm below.
    // Only a literal label can name a Callgrind marker at compile time.
    ($label:literal, $expr:expr) => {{
        $crate::__unique_label!(function, $label);
        $crate::measure_block!({ $label }, $crate::__callgrind_block!($label, $expr))
    }};

    ($label:expr, $expr:expr) => {{
        let __hotpath_label: &'static str = $label;
        {
            static __HOTPATH_LOC: $crate::Location = $crate::Location {
                file: file!(),
                line: line!(),
                column: column!(),
            };
            // The label is a runtime expression, so one call site can produce
            // several distinct labels; re-register whenever the value changes
            // (tracked by pointer) instead of only once, so every label
            // resolves to this location while the common fixed-label case
            // stays one relaxed load per execution.
            static __HOTPATH_LAST_LABEL: std::sync::atomic::AtomicPtr<u8> =
                std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
            let __hotpath_label_ptr = __hotpath_label.as_ptr() as *mut u8;
            if __HOTPATH_LAST_LABEL.load(std::sync::atomic::Ordering::Relaxed)
                != __hotpath_label_ptr
            {
                $crate::register_location(__hotpath_label, &__HOTPATH_LOC);
                __HOTPATH_LAST_LABEL
                    .store(__hotpath_label_ptr, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let _guard = $crate::functions::build_measurement_guard_block(__hotpath_label, false);

        $expr
    }};
}

/// Registers the call site's structured [`Location`] under the identity
/// string used for stats aggregation. `file!()`/`line!()`/`column!()` here
/// resolve to the user's call site because macro expansion attributes them to
/// the outermost invocation.
#[doc(hidden)]
#[macro_export]
macro_rules! __register_location {
    ($id:expr) => {{
        static __HOTPATH_LOC: $crate::Location = $crate::Location {
            file: file!(),
            line: line!(),
            column: column!(),
        };
        static __HOTPATH_LOC_ONCE: std::sync::Once = std::sync::Once::new();
        __HOTPATH_LOC_ONCE.call_once(|| $crate::register_location($id, &__HOTPATH_LOC));
    }};
}

// Build-time label uniqueness check; see the proc macro's docs.
#[doc(hidden)]
pub use hotpath_macros::__unique_label;

// Callgrind marker around a `measure_block!` body; see the proc macro's docs.
#[doc(hidden)]
pub use hotpath_macros::__callgrind_block;

/// Debug macro that tracks debug output in the profiler.
///
/// Works like `std::dbg!` but sends debug logs to a background worker thread
/// for tracking in the profiler. The logs can be viewed in the TUI or via
/// the HTTP API at `/debug`, `/debug/dbg/{id}/logs`, `/debug/val/{id}/logs`,
/// and `/debug/gauge/{id}/logs`.
///
/// # Variants
///
/// - `dbg!(expr)` - Returns value, logs expression + result
/// - `dbg!(a, b, c)` - Multiple expressions, returns tuple
///
/// # Examples
///
/// ```rust,ignore
/// use hotpath::dbg;
///
/// // Debug a single value
/// let x = dbg!(1 + 2);  // returns 3, logs "1 + 2 = 3"
///
/// // Debug multiple values
/// let (a, b) = dbg!(1, 2);  // returns (1, 2)
/// ```
#[macro_export]
macro_rules! dbg {
    ($val:expr $(,)?) => {{
        static DBG_ID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        let id = *DBG_ID.get_or_init(|| {
            $crate::debug::DEBUG_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        });
        const DBG_LOC: &'static str = concat!(file!(), ":", line!(), ":", column!());
        const DBG_EXPR: &'static str = stringify!($val);
        $crate::__register_location!(DBG_LOC);
        match $val {
            tmp => {
                $crate::debug::dbg::log_dbg(id, DBG_LOC, DBG_EXPR, &tmp);
                tmp
            }
        }
    }};
    ($($val:expr),+ $(,)?) => {
        ($($crate::dbg!($val)),+,)
    };
}

/// Value tracking macro that logs key-value pairs to the profiler.
///
/// Unlike `dbg!`, this macro takes a string key and returns a handle
/// with a `.set()` method. Values are grouped by key (not source location),
/// but each log entry records its source location for debugging.
///
/// # Examples
///
/// ```rust,ignore
/// use hotpath::val;
///
/// // Track a counter value
/// hotpath::val!("counter").set(&count);
///
/// // Track state changes
/// hotpath::val!("state").set(&current_state);
///
/// // Dynamic keys work too
/// let key = format!("counter_{}", id);
/// hotpath::val!(key).set(&value);
/// ```
#[macro_export]
macro_rules! val {
    ($key:expr) => {{
        const VAL_LOC: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(VAL_LOC);
        $crate::debug::val::ValHandle::new($key, VAL_LOC)
    }};
}

/// Gauge macro for tracking numeric values with set/inc/dec operations.
///
/// Returns a `GaugeHandle` that can be used to set, increment, or decrement
/// a numeric value. Gauges track the current value, min/max values, and
/// update history. Gauges are displayed in the Debug tab of the TUI.
///
/// # Examples
///
/// ```rust,ignore
/// use hotpath::gauge;
///
/// // Set an absolute value
/// hotpath::gauge!("queue_size").set(42.0);
///
/// // Increment/decrement with fluent API
/// hotpath::gauge!("active_connections").inc(1.0);
/// hotpath::gauge!("active_connections").dec(1.0);
///
/// // Chain operations
/// hotpath::gauge!("counter").set(0.0).inc(5.0).dec(2.0);
/// ```
#[macro_export]
macro_rules! gauge {
    ($key:expr) => {{
        const GAUGE_LOC: &'static str = concat!(file!(), ":", line!(), ":", column!());
        $crate::__register_location!(GAUGE_LOC);
        $crate::debug::gauge::GaugeHandle::new($key, GAUGE_LOC)
    }};
}

/// Initialize Tokio runtime metrics monitoring.
///
/// # Variants
///
/// - `tokio_runtime!()` - uses `tokio::runtime::Handle::current()`
/// - `tokio_runtime!($handle)` - uses the provided `&Handle`
#[macro_export]
macro_rules! tokio_runtime {
    () => {
        $crate::tokio_runtime::init_runtime_monitoring(&tokio::runtime::Handle::current());
    };
    ($handle:expr) => {
        $crate::tokio_runtime::init_runtime_monitoring($handle);
    };
}

#[cfg(test)]
mod tests {
    use crate::lib_on::HotpathGuard;

    fn is_send_sync<T: Send + Sync>() {}

    #[test]
    fn test_hotpath_is_send_sync() {
        is_send_sync::<HotpathGuard>();
    }
}
