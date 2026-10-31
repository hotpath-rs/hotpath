use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::LazyLock;

use crate::instant::Instant;

#[derive(Debug, Default)]
pub(crate) struct AsyncAllocBridge {
    bytes_total: AtomicU64,
    count_total: AtomicU64,
}

impl AsyncAllocBridge {
    #[inline]
    pub(crate) fn add(&self, bytes: u64, count: u64) {
        self.bytes_total.fetch_add(bytes, Ordering::Relaxed);
        self.count_total.fetch_add(count, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn snapshot(&self) -> (Option<u64>, Option<u64>) {
        (
            Some(self.bytes_total.load(Ordering::Relaxed)),
            Some(self.count_total.load(Ordering::Relaxed)),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AllocMetric {
    Bytes,
    Count,
}

pub(crate) static ALLOC_METRIC: LazyLock<AllocMetric> =
    LazyLock::new(|| match std::env::var("HOTPATH_ALLOC_METRIC") {
        Ok(v) => match v.to_lowercase().as_str() {
            "bytes" => AllocMetric::Bytes,
            "count" => AllocMetric::Count,
            other => panic!(
                "Invalid HOTPATH_ALLOC_METRIC value: '{}'. Expected 'bytes' or 'count'.",
                other
            ),
        },
        Err(_) => AllocMetric::Bytes,
    });

pub(crate) static ALLOC_CUMULATIVE: LazyLock<bool> =
    LazyLock::new(|| crate::shared::env_flag("HOTPATH_ALLOC_CUMULATIVE"));

#[inline]
pub(crate) fn push_alloc_stack() {
    crate::functions::alloc::core::ALLOCATIONS.with(|stack| {
        let max_depth = crate::functions::alloc::core::MAX_DEPTH;
        let depth = stack.depth.get() as usize + 1;
        // Checked before `depth` moves: the panic allocates, and
        // `track_alloc` must still index a valid frame.
        if depth > max_depth {
            panic!(
                "hotpath: more than {max_depth} nested measured functions on one thread in alloc mode. \
                 Raise the limit by rebuilding with HOTPATH_ALLOC_MAX_DEPTH set (e.g. HOTPATH_ALLOC_MAX_DEPTH=256)."
            );
        }
        stack.depth.set(depth as u32);
        stack.elements[depth].bytes_total.set(0);
        stack.elements[depth].count_total.set(0);
    });
}

#[inline]
pub(crate) fn pop_alloc_stack() -> (u64, u64) {
    crate::functions::alloc::core::ALLOCATIONS.with(|stack| {
        assert!(stack.depth.get() > 0, "pop_alloc_stack called with depth 0");
        let depth = stack.depth.get() as usize;
        let bytes = stack.elements[depth].bytes_total.get();
        let count = stack.elements[depth].count_total.get();

        stack.depth.set(stack.depth.get() - 1);

        if *ALLOC_CUMULATIVE {
            let parent = stack.depth.get() as usize;
            stack.elements[parent]
                .bytes_total
                .set(stack.elements[parent].bytes_total.get() + bytes);
            stack.elements[parent]
                .count_total
                .set(stack.elements[parent].count_total.get() + count);
        }

        (bytes, count)
    })
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn send_alloc_measurement(
    name: &'static str,
    bytes_total: Option<u64>,
    count_total: Option<u64>,
    duration_ns: Option<u64>,
    elapsed_since_start_ns: u64,
    wrapper: bool,
    tid: Option<u64>,
) {
    crate::functions::alloc::state::send_alloc_measurement(
        name,
        bytes_total,
        count_total,
        duration_ns,
        elapsed_since_start_ns,
        wrapper,
        tid,
    );
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn send_alloc_measurement_with_log(
    name: &'static str,
    bytes_total: Option<u64>,
    count_total: Option<u64>,
    duration_ns: Option<u64>,
    elapsed_since_start_ns: u64,
    wrapper: bool,
    tid: Option<u64>,
    result_log: Option<String>,
) {
    crate::functions::alloc::state::send_alloc_measurement_with_log(
        name,
        bytes_total,
        count_total,
        duration_ns,
        elapsed_since_start_ns,
        wrapper,
        tid,
        result_log,
    );
}

/// Wrapper guards are never sampled - their exact total is the `%` denominator.
/// Unsampled guards keep exact allocation tracking; only the duration is skipped.
#[inline]
fn sampled_start(wrapper: bool, skipped: bool) -> Option<Instant> {
    if skipped {
        return None;
    }
    if wrapper || crate::lib_on::sampling::functions_should_time() {
        Some(Instant::now())
    } else {
        None
    }
}

#[must_use = "guard is dropped immediately without measuring anything"]
pub struct MeasurementGuardSync {
    name: &'static str,
    wrapper: bool,
    tid: u64,
    start: Option<Instant>,
    skipped: bool,
    caller_scoped: bool,
}

impl MeasurementGuardSync {
    #[inline]
    pub fn new(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        Self::build(name, wrapper, skipped, false)
    }

    /// Also registers `name` on the thread-local caller stack for SQL/HTTP
    /// source attribution (skipped for wrapper guards).
    #[inline]
    pub(crate) fn new_caller_scoped(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        Self::build(name, wrapper, skipped, !wrapper && !skipped)
    }

    #[inline]
    fn build(name: &'static str, wrapper: bool, skipped: bool, caller_scoped: bool) -> Self {
        let _suspend = crate::lib_on::SuspendAllocTracking::new();
        if !skipped {
            push_alloc_stack();
        }
        if caller_scoped {
            crate::lib_on::caller_stack::push_caller(name);
        }

        Self {
            name,
            wrapper,
            tid: crate::tid::current_tid(),
            start: sampled_start(wrapper, skipped),
            skipped,
            caller_scoped,
        }
    }
}

impl Drop for MeasurementGuardSync {
    #[inline]
    fn drop(&mut self) {
        if self.skipped {
            return;
        }

        let _suspend = crate::lib_on::SuspendAllocTracking::new();
        let end = Instant::now();
        let duration_ns = self
            .start
            .map(|start| end.duration_since(start).as_nanos() as u64);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let cross_thread = crate::tid::current_tid() != self.tid;

        if self.caller_scoped && !cross_thread {
            crate::lib_on::caller_stack::pop_caller();
        }
        let (bytes_total, count_total) = if cross_thread {
            (None, None)
        } else {
            let (bytes, count) = pop_alloc_stack();
            (Some(bytes), Some(count))
        };

        send_alloc_measurement(
            self.name,
            bytes_total,
            count_total,
            duration_ns,
            elapsed_since_start_ns,
            self.wrapper,
            Some(self.tid),
        );
    }
}

#[must_use = "guard is dropped immediately without measuring anything"]
pub struct MeasurementGuardAsync {
    name: &'static str,
    wrapper: bool,
    tid: u64,
    start: Option<Instant>,
    skipped: bool,
    alloc_bridge: Option<Arc<AsyncAllocBridge>>,
}

impl MeasurementGuardAsync {
    #[inline]
    pub(crate) fn new(
        name: &'static str,
        wrapper: bool,
        skipped: bool,
        alloc_bridge: Option<Arc<AsyncAllocBridge>>,
    ) -> Self {
        Self {
            name,
            wrapper,
            tid: crate::tid::current_tid(),
            start: sampled_start(wrapper, skipped),
            skipped,
            alloc_bridge,
        }
    }
}

impl Drop for MeasurementGuardAsync {
    #[inline]
    fn drop(&mut self) {
        if self.skipped {
            return;
        }

        let _suspend = crate::lib_on::SuspendAllocTracking::new();
        let end = Instant::now();
        let duration_ns = self
            .start
            .map(|start| end.duration_since(start).as_nanos() as u64);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let (bytes_total, count_total) = self
            .alloc_bridge
            .as_ref()
            .map_or((None, None), |bridge| bridge.snapshot());

        send_alloc_measurement(
            self.name,
            bytes_total,
            count_total,
            duration_ns,
            elapsed_since_start_ns,
            self.wrapper,
            Some(self.tid),
        );
    }
}

#[must_use = "guard is dropped immediately without measuring anything"]
pub(crate) struct MeasurementGuardSyncWithLog {
    name: &'static str,
    wrapper: bool,
    tid: u64,
    start: Option<Instant>,
    finished: bool,
    skipped: bool,
    caller_scoped: bool,
}

impl MeasurementGuardSyncWithLog {
    /// Also registers `name` on the thread-local caller stack for SQL/HTTP
    /// source attribution (skipped for wrapper guards).
    #[inline]
    pub(crate) fn new_caller_scoped(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        let _suspend = crate::lib_on::SuspendAllocTracking::new();
        let caller_scoped = !wrapper && !skipped;
        if !skipped {
            push_alloc_stack();
        }
        if caller_scoped {
            crate::lib_on::caller_stack::push_caller(name);
        }

        Self {
            name,
            wrapper,
            tid: crate::tid::current_tid(),
            start: sampled_start(wrapper, skipped),
            finished: false,
            skipped,
            caller_scoped,
        }
    }

    #[inline]
    pub(crate) fn finish_with_result<T: std::fmt::Debug>(mut self, result: &T) {
        self.finished = true;
        if self.skipped {
            return;
        }

        let _suspend = crate::lib_on::SuspendAllocTracking::new();
        let end = Instant::now();
        let duration_ns = self
            .start
            .map(|start| end.duration_since(start).as_nanos() as u64);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let result_str = crate::output_on::format_debug_truncated(result);
        let cross_thread = crate::tid::current_tid() != self.tid;

        if self.caller_scoped && !cross_thread {
            crate::lib_on::caller_stack::pop_caller();
        }
        let (bytes_total, count_total) = if cross_thread {
            (None, None)
        } else {
            let (bytes, count) = pop_alloc_stack();
            (Some(bytes), Some(count))
        };

        send_alloc_measurement_with_log(
            self.name,
            bytes_total,
            count_total,
            duration_ns,
            elapsed_since_start_ns,
            self.wrapper,
            Some(self.tid),
            Some(result_str),
        );
    }
}

impl Drop for MeasurementGuardSyncWithLog {
    #[inline]
    fn drop(&mut self) {
        if self.skipped || self.finished {
            return;
        }

        let _suspend = crate::lib_on::SuspendAllocTracking::new();
        let end = Instant::now();
        let duration_ns = self
            .start
            .map(|start| end.duration_since(start).as_nanos() as u64);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let cross_thread = crate::tid::current_tid() != self.tid;

        if self.caller_scoped && !cross_thread {
            crate::lib_on::caller_stack::pop_caller();
        }
        let (bytes_total, count_total) = if cross_thread {
            (None, None)
        } else {
            let (bytes, count) = pop_alloc_stack();
            (Some(bytes), Some(count))
        };

        send_alloc_measurement_with_log(
            self.name,
            bytes_total,
            count_total,
            duration_ns,
            elapsed_since_start_ns,
            self.wrapper,
            Some(self.tid),
            None,
        );
    }
}

#[must_use = "guard is dropped immediately without measuring anything"]
pub(crate) struct MeasurementGuardAsyncWithLog {
    name: &'static str,
    wrapper: bool,
    tid: u64,
    start: Option<Instant>,
    finished: bool,
    skipped: bool,
    alloc_bridge: Option<Arc<AsyncAllocBridge>>,
}

impl MeasurementGuardAsyncWithLog {
    #[inline]
    pub(crate) fn new(
        name: &'static str,
        wrapper: bool,
        skipped: bool,
        alloc_bridge: Option<Arc<AsyncAllocBridge>>,
    ) -> Self {
        Self {
            name,
            wrapper,
            tid: crate::tid::current_tid(),
            start: sampled_start(wrapper, skipped),
            finished: false,
            skipped,
            alloc_bridge,
        }
    }

    #[inline]
    pub(crate) fn finish_with_result<T: std::fmt::Debug>(mut self, result: &T) {
        self.finished = true;
        if self.skipped {
            return;
        }

        let _suspend = crate::lib_on::SuspendAllocTracking::new();
        let end = Instant::now();
        let duration_ns = self
            .start
            .map(|start| end.duration_since(start).as_nanos() as u64);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let result_str = crate::output_on::format_debug_truncated(result);
        let (bytes_total, count_total) = self
            .alloc_bridge
            .as_ref()
            .map_or((None, None), |bridge| bridge.snapshot());

        send_alloc_measurement_with_log(
            self.name,
            bytes_total,
            count_total,
            duration_ns,
            elapsed_since_start_ns,
            self.wrapper,
            Some(self.tid),
            Some(result_str),
        );
    }
}

impl Drop for MeasurementGuardAsyncWithLog {
    #[inline]
    fn drop(&mut self) {
        if self.skipped || self.finished {
            return;
        }

        let _suspend = crate::lib_on::SuspendAllocTracking::new();
        let end = Instant::now();
        let duration_ns = self
            .start
            .map(|start| end.duration_since(start).as_nanos() as u64);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let (bytes_total, count_total) = self
            .alloc_bridge
            .as_ref()
            .map_or((None, None), |bridge| bridge.snapshot());

        send_alloc_measurement_with_log(
            self.name,
            bytes_total,
            count_total,
            duration_ns,
            elapsed_since_start_ns,
            self.wrapper,
            Some(self.tid),
            None,
        );
    }
}

#[cfg(all(test, feature = "hotpath-alloc"))]
mod tests {
    use crate::functions::alloc::core::{track_alloc, MAX_DEPTH};
    use crate::functions::alloc::guard::{pop_alloc_stack, push_alloc_stack};

    #[test]
    fn nesting_up_to_max_depth_keeps_frames_exclusive() {
        for _ in 0..MAX_DEPTH {
            push_alloc_stack();
        }
        track_alloc(64);
        assert_eq!(pop_alloc_stack(), (64, 1));
        for _ in 1..MAX_DEPTH {
            pop_alloc_stack();
        }
    }

    #[test]
    #[should_panic(expected = "HOTPATH_ALLOC_MAX_DEPTH")]
    fn nesting_past_max_depth_panics_with_hint() {
        for _ in 0..=MAX_DEPTH {
            push_alloc_stack();
        }
    }
}
