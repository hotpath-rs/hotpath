use crate::instant::Instant;

#[cfg(feature = "hotpath-sink-meta")]
use crate::lib_on::functions::sink::SinkScope;
use crate::output_on::format_debug_truncated;
#[cfg(feature = "hotpath-sink-meta")]
use crate::ScopeKind;

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
    #[cfg(feature = "hotpath-sink-meta")]
    sink: Option<SinkScope>,
}

impl MeasurementGuard {
    #[inline]
    pub fn new(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        Self::build(name, wrapper, skipped, false)
    }

    /// Sync-only constructor: registers `name` on the thread-local caller
    /// stack for SQL/HTTP source attribution. Must not be used for guards
    /// held across `.await` points - async bodies get per-poll scoping via
    /// `futures::wrapper` instead.
    #[inline]
    pub(crate) fn new_caller_scoped(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        Self::build(name, wrapper, skipped, !wrapper && !skipped)
    }

    #[inline]
    fn build(name: &'static str, wrapper: bool, skipped: bool, caller_scoped: bool) -> Self {
        if caller_scoped {
            crate::lib_on::caller_stack::push_caller(name);
        }
        // Entered before the clock read so hotpath's own timing stays inside
        // the sink scope and never includes the sink call.
        #[cfg(feature = "hotpath-sink-meta")]
        let sink = SinkScope::enter(name, ScopeKind::of_guard(caller_scoped), skipped);
        Self {
            name,
            start: sampled_start(wrapper, skipped),
            wrapper,
            tid: if skipped {
                0
            } else {
                crate::tid::current_tid()
            },
            skipped,
            caller_scoped,
            #[cfg(feature = "hotpath-sink-meta")]
            sink,
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
        #[cfg(feature = "hotpath-sink-meta")]
        if let Some(sink) = self.sink.take() {
            sink.exit(self.name);
        }
        let duration_ns = self
            .start
            .map(|start| end.duration_since(start).as_nanos() as u64);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let cross_thread = crate::tid::current_tid() != self.tid;
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
#[doc(hidden)]
pub struct MeasurementGuardWithLog {
    name: &'static str,
    start: Option<Instant>,
    wrapper: bool,
    tid: u64,
    finished: bool,
    skipped: bool,
    caller_scoped: bool,
    #[cfg(feature = "hotpath-sink-meta")]
    sink: Option<SinkScope>,
}

impl MeasurementGuardWithLog {
    #[inline]
    pub fn new(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        Self::build(name, wrapper, skipped, false)
    }

    /// Sync-only constructor: registers `name` on the thread-local caller
    /// stack for SQL/HTTP source attribution. Must not be used for guards
    /// held across `.await` points.
    #[inline]
    pub(crate) fn new_caller_scoped(name: &'static str, wrapper: bool, skipped: bool) -> Self {
        Self::build(name, wrapper, skipped, !wrapper && !skipped)
    }

    #[inline]
    fn build(name: &'static str, wrapper: bool, skipped: bool, caller_scoped: bool) -> Self {
        if caller_scoped {
            crate::lib_on::caller_stack::push_caller(name);
        }
        // Entered before the clock read so hotpath's own timing stays inside
        // the sink scope and never includes the sink call.
        #[cfg(feature = "hotpath-sink-meta")]
        let sink = SinkScope::enter(name, ScopeKind::of_guard(caller_scoped), skipped);
        Self {
            name,
            start: sampled_start(wrapper, skipped),
            wrapper,
            tid: if skipped {
                0
            } else {
                crate::tid::current_tid()
            },
            finished: false,
            skipped,
            caller_scoped,
            #[cfg(feature = "hotpath-sink-meta")]
            sink,
        }
    }

    #[inline]
    pub fn finish_with_result<T: std::fmt::Debug>(mut self, result: &T) {
        self.finished = true;
        if self.skipped {
            return;
        }
        let end = Instant::now();
        #[cfg(feature = "hotpath-sink-meta")]
        if let Some(sink) = self.sink.take() {
            sink.exit(self.name);
        }
        let duration_ns = self
            .start
            .map(|start| end.duration_since(start).as_nanos() as u64);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let result_str = Some(format_debug_truncated(result));
        let cross_thread = crate::tid::current_tid() != self.tid;
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
        #[cfg(feature = "hotpath-sink-meta")]
        if let Some(sink) = self.sink.take() {
            sink.exit(self.name);
        }
        let duration_ns = self
            .start
            .map(|start| end.duration_since(start).as_nanos() as u64);
        let elapsed_since_start_ns = crate::lib_on::elapsed_since_start_ns(end);
        let cross_thread = crate::tid::current_tid() != self.tid;
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
