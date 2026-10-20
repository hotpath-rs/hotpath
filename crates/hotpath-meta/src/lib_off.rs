pub use hotpath_macros_meta::{future_fn, main, measure, measure_all, skip};

#[macro_export]
macro_rules! measure_block {
    ($label:expr, $expr:expr) => {{
        $expr
    }};
}

#[macro_export]
macro_rules! dbg {
    ($val:expr $(,)?) => {
        match $val {
            tmp => tmp
        }
    };
    ($($val:expr),+ $(,)?) => {
        ($($crate::dbg!($val)),+,)
    };
}

#[macro_export]
macro_rules! val {
    ($key:expr) => {{
        $crate::ValHandle
    }};
}

/// No-op counterpart of the tracking allocator: a pure pass-through to the
/// inner allocator, so a `#[global_allocator]` declaration compiles unchanged
/// with the `hotpath-meta` feature disabled.
pub struct CountingAllocator<A = std::alloc::System>(A);

impl CountingAllocator<std::alloc::System> {
    pub const fn new() -> Self {
        Self(std::alloc::System)
    }
}

impl<A> CountingAllocator<A> {
    pub const fn with(inner: A) -> Self {
        Self(inner)
    }
}

impl<A: Default> Default for CountingAllocator<A> {
    fn default() -> Self {
        Self(A::default())
    }
}

// SAFETY: pure pass-through - every pointer returned by
// `alloc`/`alloc_zeroed`/`realloc` comes from the corresponding method of `A`,
// and `dealloc`/`realloc` forward the caller's `ptr`/`layout`/`new_size`
// unchanged, so `A`'s GlobalAlloc guarantees carry over.
unsafe impl<A> std::alloc::GlobalAlloc for CountingAllocator<A>
where
    A: std::alloc::GlobalAlloc,
{
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        // SAFETY: caller upholds GlobalAlloc's contract for `layout`; it is
        // forwarded unchanged.
        unsafe { self.0.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: caller guarantees `ptr` was allocated by this allocator
        // with `layout`, which means it came from `A::alloc`; both are
        // forwarded unchanged.
        unsafe {
            self.0.dealloc(ptr, layout);
        }
    }

    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        // SAFETY: caller upholds GlobalAlloc's contract for `layout`; it is
        // forwarded unchanged.
        unsafe { self.0.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        // SAFETY: caller guarantees `ptr` was allocated by this allocator
        // with `layout` and that `new_size` is valid per GlobalAlloc's
        // contract; all three are forwarded unchanged.
        unsafe { self.0.realloc(ptr, layout, new_size) }
    }
}

pub struct ValHandle;

impl ValHandle {
    #[inline]
    pub fn set<T: std::fmt::Debug>(&self, _value: &T) {}
}

use crate::shared::IntoF64;

#[macro_export]
macro_rules! gauge {
    ($key:expr) => {{
        $crate::GaugeHandle
    }};
}

pub struct GaugeHandle;

impl GaugeHandle {
    #[inline]
    pub fn set(&self, _value: impl IntoF64) -> &Self {
        self
    }

    #[inline]
    pub fn inc(&self, _delta: impl IntoF64) -> &Self {
        self
    }

    #[inline]
    pub fn dec(&self, _delta: impl IntoF64) -> &Self {
        self
    }
}

#[macro_export]
macro_rules! channel {
    // Profiling disabled: every form (`label`, `log`, `capacity`, `iter`, any order)
    // returns the original channel unchanged.
    ($expr:expr $(, $($rest:tt)*)?) => {
        $expr
    };
}

#[macro_export]
macro_rules! stream {
    // Profiling disabled: every form (`label`, `log`, `iter`, any order)
    // returns the original stream unchanged.
    ($expr:expr $(, $($rest:tt)*)?) => {
        $expr
    };
}

#[macro_export]
macro_rules! tokio_runtime {
    ($($handle:expr)?) => {};
}

#[macro_export]
macro_rules! future {
    ($fut:expr) => {
        $fut
    };
    ($fut:expr, label = $label:expr) => {
        $fut
    };
    ($fut:expr, log = true) => {
        $fut
    };
    ($fut:expr, label = $label:expr, log = true) => {
        $fut
    };
    ($fut:expr, log = true, label = $label:expr) => {
        $fut
    };
}

use crate::Format;
use crate::Section;

#[inline]
pub fn measure_sync_log<T: std::fmt::Debug, F: FnOnce() -> T>(
    _measurement_loc: &'static str,
    f: F,
) -> T {
    f()
}

pub async fn measure_async<T, Fut>(_measurement_loc: &'static str, fut: Fut) -> T
where
    Fut: std::future::Future<Output = T>,
{
    fut.await
}

pub async fn measure_async_log<T: std::fmt::Debug, Fut>(
    _measurement_loc: &'static str,
    fut: Fut,
) -> T
where
    Fut: std::future::Future<Output = T>,
{
    fut.await
}

pub async fn measure_async_future<T, Fut>(_measurement_loc: &'static str, fut: Fut) -> T
where
    Fut: std::future::Future<Output = T>,
{
    fut.await
}

pub async fn measure_async_future_log<T: std::fmt::Debug, Fut>(
    _measurement_loc: &'static str,
    fut: Fut,
) -> T
where
    Fut: std::future::Future<Output = T>,
{
    fut.await
}

pub struct HotpathGuard;

impl Default for HotpathGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl HotpathGuard {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

pub struct HotpathGuardBuilder {}

impl HotpathGuardBuilder {
    pub fn new(_caller_name: &'static str) -> Self {
        Self {}
    }

    pub fn percentiles(self, _percentiles: &[f64]) -> Self {
        self
    }

    pub fn format(self, _format: Format) -> Self {
        self
    }

    pub fn functions_limit(self, _limit: usize) -> Self {
        self
    }

    pub fn channels_limit(self, _limit: usize) -> Self {
        self
    }

    pub fn streams_limit(self, _limit: usize) -> Self {
        self
    }

    pub fn futures_limit(self, _limit: usize) -> Self {
        self
    }

    pub fn threads_limit(self, _limit: usize) -> Self {
        self
    }

    pub fn rw_locks_limit(self, _limit: usize) -> Self {
        self
    }

    pub fn mutexes_limit(self, _limit: usize) -> Self {
        self
    }

    pub fn sql_limit(self, _limit: usize) -> Self {
        self
    }

    pub fn http_limit(self, _limit: usize) -> Self {
        self
    }

    pub fn server_limit(self, _limit: usize) -> Self {
        self
    }

    pub fn io_limit(self, _limit: usize) -> Self {
        self
    }

    #[cfg(feature = "axum-0-8")]
    pub fn route_scope(self, _enabled: bool) -> Self {
        self
    }

    pub fn limit(self, _limit: usize) -> Self {
        self
    }

    pub fn time_sampling_rate(self, _rate: f64) -> Self {
        self
    }

    pub fn functions_time_sampling_rate(self, _rate: f64) -> Self {
        self
    }

    pub fn mutexes_time_sampling_rate(self, _rate: f64) -> Self {
        self
    }

    pub fn rw_locks_time_sampling_rate(self, _rate: f64) -> Self {
        self
    }

    pub fn futures_time_sampling_rate(self, _rate: f64) -> Self {
        self
    }

    pub fn channels_time_sampling_rate(self, _rate: f64) -> Self {
        self
    }

    pub fn io_time_sampling_rate(self, _rate: f64) -> Self {
        self
    }

    pub fn output_path(self, _path: impl AsRef<std::path::Path>) -> Self {
        self
    }

    pub fn sections(self, _sections: Vec<Section>) -> Self {
        self
    }

    pub fn sections_exclude(self, _sections: Vec<Section>) -> Self {
        self
    }

    pub fn report(self, _spec: &str) -> Self {
        self
    }

    pub fn before_shutdown(self, _f: impl FnOnce() + Send + 'static) -> Self {
        self
    }

    pub fn user_metadata(self, _metadata: std::collections::HashMap<String, String>) -> Self {
        self
    }

    pub fn build(self) -> HotpathGuard {
        HotpathGuard
    }

    pub fn build_with_shutdown(self, _duration: std::time::Duration) {}
}

#[macro_export]
macro_rules! axum {
    ($router:expr) => {
        $router
    };
}

/// No-op tower layer used when the `hotpath-meta` feature is disabled. Lets
/// call sites keep `.layer(hotpath_meta::AxumLayer::new())` in their router
/// setup unconditionally - it returns the inner service untouched.
#[cfg(feature = "axum-0-8")]
#[derive(Default, Clone, Copy, Debug)]
pub struct AxumLayer;

#[cfg(feature = "axum-0-8")]
impl AxumLayer {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(feature = "axum-0-8")]
impl<S> tower_layer::Layer<S> for AxumLayer {
    type Service = S;

    fn layer(&self, inner: S) -> Self::Service {
        inner
    }
}

#[macro_export]
macro_rules! rw_lock {
    ($expr:expr) => {
        $expr
    };
    ($expr:expr, label = $label:expr) => {
        $expr
    };
}

#[macro_export]
macro_rules! mutex {
    ($expr:expr) => {
        $expr
    };
    ($expr:expr, label = $label:expr) => {
        $expr
    };
}

/// No-op counterpart of the enabled-mode `io_unwrap`. `io!` returns its
/// argument unchanged when profiling is disabled, so unwrapping is the
/// identity and call sites compile identically in both modes.
pub fn io_unwrap<T>(io: T) -> T {
    io
}

#[macro_export]
macro_rules! io {
    ($expr:expr) => {
        $expr
    };
    ($expr:expr, label = $label:expr) => {
        $expr
    };
    ($expr:expr, iter = true) => {
        $expr
    };
    ($expr:expr, label = $label:expr, iter = true) => {
        $expr
    };
    ($expr:expr, iter = true, label = $label:expr) => {
        $expr
    };
}
