use std::sync::Arc;

use crate::instant::Instant;
use crate::lib_on::functions::async_bridge::AsyncCallBridge;
use crate::lib_on::functions::exclusive::{self, AsyncCall, ExclusiveFrame};

use crate::output_on::format_debug_truncated;

/// What a guard needs to report its duration in exclusive mode
/// (`HOTPATH_TIME_EXCLUSIVE`).
enum CallTime {
    /// Sync function or block: a frame on the thread that built the guard,
    /// `None` in inclusive mode.
    Sync(Option<ExclusiveFrame>),
    /// Async function: the body's polls report through the call's bridge.
    Async(AsyncCall),
}

impl CallTime {
    #[inline]
    fn open_sync(skipped: bool) -> Self {
        Self::Sync(if skipped {
            None
        } else {
            ExclusiveFrame::enter()
        })
    }

    /// Called once the guard knows its start time.
    #[inline]
    fn started(mut self, start: Option<Instant>) -> Self {
        if let Self::Async(call) = &mut self {
            call.started(start);
        }
        self
    }

    #[inline]
    fn duration_ns(
        &mut self,
        start: Option<Instant>,
        end: Instant,
        cross_thread: bool,
    ) -> Option<u64> {
        match self {
            Self::Sync(frame) => {
                exclusive::sync_duration_ns(frame.take(), start, end, cross_thread)
            }
            Self::Async(call) => call.duration_ns(start, end),
        }
    }
}

/// Wrapper guards are never sampled - their exact total is the `%` denominator.
/// Unsampled guards skip the start clock read and send a `None` duration;
/// elapsed time and result logs stay exact.
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

#[doc(hidden)]
#[must_use = "guard is dropped immediately without measuring anything"]
pub struct MeasurementGuard {
    name: &'static str,
    start: Option<Instant>,
    wrapper: bool,
    tid: u64,
    skipped: bool,
    caller_scoped: bool,
    call_time: CallTime,
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl MeasurementGuard {
    #[inline]
    pub fn new(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        Self::build(name, wrapper, skipped, false, CallTime::open_sync(skipped))
    }

    /// Async-only constructor: the guard lives across `.await` points, so it
    /// opens no exclusive frame and reads the body's polls from `bridge`.
    #[inline]
    pub(crate) fn new_async(
        name: &'static str,
        wrapper: bool,
        skipped: bool,
        bridge: Option<Arc<AsyncCallBridge>>,
    ) -> Self {
        Self::build(
            name,
            wrapper,
            skipped,
            false,
            CallTime::Async(AsyncCall::new(bridge)),
        )
    }

    /// Sync-only constructor: registers `name` on the thread-local caller
    /// stack for SQL/HTTP source attribution. Must not be used for guards
    /// held across `.await` points - async bodies get per-poll scoping via
    /// `futures::wrapper` instead.
    #[inline]
    pub(crate) fn new_caller_scoped(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        Self::build(
            name,
            wrapper,
            skipped,
            !wrapper && !skipped,
            CallTime::open_sync(skipped),
        )
    }

    #[inline]
    fn build(
        name: &'static str,
        wrapper: bool,
        skipped: bool,
        caller_scoped: bool,
        call_time: CallTime,
    ) -> Self {
        if caller_scoped {
            crate::lib_on::caller_stack::push_caller(name);
        }
        let start = sampled_start(wrapper, skipped);
        Self {
            name,
            start,
            wrapper,
            tid: if skipped {
                0
            } else {
                crate::tid::current_tid()
            },
            skipped,
            caller_scoped,
            call_time: call_time.started(start),
        }
    }
}

impl Drop for MeasurementGuard {
    #[inline]
    fn drop(&mut self) {
        if self.skipped {
            return;
        }
        let end = Instant::now();
        let cross_thread = crate::tid::current_tid() != self.tid;
        let duration_ns = self.call_time.duration_ns(self.start, end, cross_thread);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        if self.caller_scoped && !cross_thread {
            crate::lib_on::caller_stack::pop_caller();
        }
        let tid = if cross_thread { None } else { Some(self.tid) };
        crate::lib_on::functions::timing::state::send_duration_measurement(
            self.name,
            duration_ns,
            elapsed_since_start_ns,
            self.wrapper,
            tid,
        );
    }
}

#[doc(hidden)]
#[must_use = "guard is dropped immediately without measuring anything"]
pub(crate) struct MeasurementGuardWithLog {
    name: &'static str,
    start: Option<Instant>,
    wrapper: bool,
    tid: u64,
    finished: bool,
    skipped: bool,
    caller_scoped: bool,
    call_time: CallTime,
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl MeasurementGuardWithLog {
    /// Async-only constructor: the guard lives across `.await` points, so it
    /// opens no exclusive frame and reads the body's polls from `bridge`.
    #[inline]
    pub(crate) fn new_async(
        name: &'static str,
        wrapper: bool,
        skipped: bool,
        bridge: Option<Arc<AsyncCallBridge>>,
    ) -> Self {
        Self::build(
            name,
            wrapper,
            skipped,
            false,
            CallTime::Async(AsyncCall::new(bridge)),
        )
    }

    /// Sync-only constructor: registers `name` on the thread-local caller
    /// stack for SQL/HTTP source attribution. Must not be used for guards
    /// held across `.await` points.
    #[inline]
    pub(crate) fn new_caller_scoped(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        Self::build(
            name,
            wrapper,
            skipped,
            !wrapper && !skipped,
            CallTime::open_sync(skipped),
        )
    }

    #[inline]
    fn build(
        name: &'static str,
        wrapper: bool,
        skipped: bool,
        caller_scoped: bool,
        call_time: CallTime,
    ) -> Self {
        if caller_scoped {
            crate::lib_on::caller_stack::push_caller(name);
        }
        let start = sampled_start(wrapper, skipped);
        Self {
            name,
            start,
            wrapper,
            tid: if skipped {
                0
            } else {
                crate::tid::current_tid()
            },
            finished: false,
            skipped,
            caller_scoped,
            call_time: call_time.started(start),
        }
    }

    #[inline]
    pub fn finish_with_result<T: std::fmt::Debug>(mut self, result: &T) {
        self.finished = true;
        if self.skipped {
            return;
        }
        let end = Instant::now();
        let cross_thread = crate::tid::current_tid() != self.tid;
        let duration_ns = self.call_time.duration_ns(self.start, end, cross_thread);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let result_str = Some(format_debug_truncated(result));
        if self.caller_scoped && !cross_thread {
            crate::lib_on::caller_stack::pop_caller();
        }
        let tid = if cross_thread { None } else { Some(self.tid) };
        crate::lib_on::functions::timing::state::send_duration_measurement_with_log(
            self.name,
            duration_ns,
            elapsed_since_start_ns,
            self.wrapper,
            tid,
            result_str,
        );
    }
}

impl Drop for MeasurementGuardWithLog {
    #[inline]
    fn drop(&mut self) {
        if self.skipped || self.finished {
            return;
        }
        let end = Instant::now();
        let cross_thread = crate::tid::current_tid() != self.tid;
        let duration_ns = self.call_time.duration_ns(self.start, end, cross_thread);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        if self.caller_scoped && !cross_thread {
            crate::lib_on::caller_stack::pop_caller();
        }
        let tid = if cross_thread { None } else { Some(self.tid) };
        crate::lib_on::functions::timing::state::send_duration_measurement_with_log(
            self.name,
            duration_ns,
            elapsed_since_start_ns,
            self.wrapper,
            tid,
            None,
        );
    }
}
