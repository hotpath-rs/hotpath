use std::sync::Arc;
use std::sync::LazyLock;

use crate::instant::Instant;
use crate::lib_on::functions::async_bridge::AsyncCallBridge;
use crate::lib_on::functions::exclusive::{self, AsyncCall, ExclusiveFrame};

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
        let current_depth = stack.depth.get();
        stack.depth.set(current_depth + 1);
        assert!((stack.depth.get() as usize) < crate::functions::alloc::core::MAX_DEPTH);
        let depth = stack.depth.get() as usize;
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
    /// Open exclusive-time frame (`HOTPATH_TIME_EXCLUSIVE` only).
    frame: Option<ExclusiveFrame>,
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
        let frame = if skipped {
            None
        } else {
            ExclusiveFrame::enter()
        };
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
            frame,
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
        let cross_thread = crate::tid::current_tid() != self.tid;
        let duration_ns =
            exclusive::sync_duration_ns(self.frame.take(), self.start, end, cross_thread);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);

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
    call: AsyncCall,
}

impl MeasurementGuardAsync {
    #[inline]
    pub(crate) fn new_async(
        name: &'static str,
        wrapper: bool,
        skipped: bool,
        bridge: Option<Arc<AsyncCallBridge>>,
    ) -> Self {
        let start = sampled_start(wrapper, skipped);
        let mut call = AsyncCall::new(bridge);
        call.started(start);
        Self {
            name,
            wrapper,
            tid: crate::tid::current_tid(),
            start,
            skipped,
            call,
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
        let duration_ns = self.call.duration_ns(self.start, end);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let (bytes_total, count_total) = self
            .call
            .bridge()
            .map_or((None, None), AsyncCallBridge::alloc_snapshot);

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
    /// Open exclusive-time frame (`HOTPATH_TIME_EXCLUSIVE` only).
    frame: Option<ExclusiveFrame>,
}

impl MeasurementGuardSyncWithLog {
    /// Also registers `name` on the thread-local caller stack for SQL/HTTP
    /// source attribution (skipped for wrapper guards).
    #[inline]
    pub(crate) fn new_caller_scoped(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        let _suspend = crate::lib_on::SuspendAllocTracking::new();
        let caller_scoped = !wrapper && !skipped;
        let frame = if skipped {
            None
        } else {
            ExclusiveFrame::enter()
        };
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
            frame,
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
        let cross_thread = crate::tid::current_tid() != self.tid;
        let duration_ns =
            exclusive::sync_duration_ns(self.frame.take(), self.start, end, cross_thread);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let result_str = crate::output_on::format_debug_truncated(result);

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
        let cross_thread = crate::tid::current_tid() != self.tid;
        let duration_ns =
            exclusive::sync_duration_ns(self.frame.take(), self.start, end, cross_thread);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);

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
    call: AsyncCall,
}

impl MeasurementGuardAsyncWithLog {
    #[inline]
    pub(crate) fn new_async(
        name: &'static str,
        wrapper: bool,
        skipped: bool,
        bridge: Option<Arc<AsyncCallBridge>>,
    ) -> Self {
        let start = sampled_start(wrapper, skipped);
        let mut call = AsyncCall::new(bridge);
        call.started(start);
        Self {
            name,
            wrapper,
            tid: crate::tid::current_tid(),
            start,
            finished: false,
            skipped,
            call,
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
        let duration_ns = self.call.duration_ns(self.start, end);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let result_str = crate::output_on::format_debug_truncated(result);
        let (bytes_total, count_total) = self
            .call
            .bridge()
            .map_or((None, None), AsyncCallBridge::alloc_snapshot);

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
        let duration_ns = self.call.duration_ns(self.start, end);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let (bytes_total, count_total) = self
            .call
            .bridge()
            .map_or((None, None), AsyncCallBridge::alloc_snapshot);

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
