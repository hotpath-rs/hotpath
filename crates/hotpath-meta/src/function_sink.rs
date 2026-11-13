//! Public types of the function sink: a callback that receives every
//! `#[measure]` / `measure_block!` scope as it is entered and exited, so an
//! external recorder (a timeline viewer, a tracing bridge) can see individual
//! calls instead of only the aggregated report.

/// How a measured scope relates to the thread that entered it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScopeKind {
    /// A sync function body. `exit` runs on the thread that called `enter`
    /// and sync scopes on one thread are strictly nested (LIFO).
    Sync,
    /// An `async fn`, a `measure_block!` or the `#[hotpath_meta::main]` wrapper.
    /// The scope may suspend across `.await` points, so `exit` can run on
    /// another thread and interleave with other scopes; use the token returned
    /// by `enter` to pair the two calls.
    Async,
}

/// Receives every measured function scope while it runs.
///
/// Install one per process with [`set_function_sink`](crate::set_function_sink).
/// Both methods run inline on the instrumented thread, so they must be cheap
/// and must never block. They are called whether or not a
/// [`HotpathGuard`](crate::HotpathGuard) is alive and regardless of time
/// sampling; scopes filtered out by `HOTPATH_META_FOCUS` are not reported.
///
/// ```
/// use std::sync::atomic::{AtomicU64, Ordering};
///
/// struct CountCalls(AtomicU64);
///
/// impl hotpath_meta::FunctionSink for CountCalls {
///     fn enter(&self, _name: &'static str, _kind: hotpath_meta::ScopeKind) -> u64 {
///         self.0.fetch_add(1, Ordering::Relaxed)
///     }
///
///     fn exit(&self, _name: &'static str, _kind: hotpath_meta::ScopeKind, _token: u64) {}
/// }
///
/// hotpath_meta::set_function_sink(CountCalls(AtomicU64::new(0))).ok();
/// ```
pub trait FunctionSink: Send + Sync + 'static {
    /// A scope named `name` (`module_path::function` or the block label)
    /// started. The returned token is handed back to the matching
    /// [`exit`](FunctionSink::exit).
    fn enter(&self, name: &'static str, kind: ScopeKind) -> u64;

    /// The scope started by the `enter` call that returned `token` finished.
    fn exit(&self, name: &'static str, kind: ScopeKind, token: u64);
}

/// Returned by [`set_function_sink`](crate::set_function_sink) when a sink is
/// already installed. The sink cannot be replaced for the lifetime of the
/// process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionSinkAlreadySet;

impl std::fmt::Display for FunctionSinkAlreadySet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a hotpath function sink is already installed")
    }
}

impl std::error::Error for FunctionSinkAlreadySet {}
