//! Instrumented Future wrapper that tracks lifecycle events.

use crate::output_on::format_debug_truncated;

use crate::functions::AsyncAllocBridge;
use crate::lib_on::futures::{
    ensure_futures_state, get_or_create_future_id, send_future_event, FutureEvent, PollResult,
    FUTURE_CALL_ID_COUNTER,
};
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::instant::Instant;

#[cfg(feature = "hotpath-alloc-meta")]
#[inline]
fn measure_poll_alloc<R>(poll_fn: impl FnOnce() -> R) -> (R, Option<u64>, Option<u64>) {
    crate::functions::alloc::guard::push_alloc_stack();

    let result = poll_fn();

    let (bytes, count) = crate::functions::alloc::guard::pop_alloc_stack();

    (result, Some(bytes), Some(count))
}

#[cfg(not(feature = "hotpath-alloc-meta"))]
#[inline]
fn measure_poll_alloc<R>(poll_fn: impl FnOnce() -> R) -> (R, Option<u64>, Option<u64>) {
    (poll_fn(), None, None)
}

pin_project! {
    /// A wrapper around a future that tracks lifecycle events.
    ///
    /// Created via the `future!` macro, this wrapper tracks:
    /// - Creation
    /// - Each poll call with result (Pending/Ready) and duration
    /// - Memory allocations per poll (when `hotpath-alloc` feature is enabled)
    /// - Drop (cancellation if not completed)
    ///
    /// This variant does NOT require `Debug` on the output type.
    /// Use `InstrumentedFutureLog` (via `future!(expr, log = true)`) to log the output value.
    pub struct InstrumentedFuture<F: Future> {
        #[pin]
        inner: F,
        future_id: u32,
        call_id: u32,
        completed: bool,
        visible: bool,
        timed: bool,
        alloc_bridge: Option<Arc<AsyncAllocBridge>>,
        // Function name registered on the thread-local caller stack for
        // exactly the duration of each inner poll (SQL/HTTP source
        // attribution). Set for measured async function bodies, None for
        // `future!` expression wrappers.
        caller_scope: Option<&'static str>,
    }

    impl<F: Future> PinnedDrop for InstrumentedFuture<F> {
        fn drop(this: Pin<&mut Self>) {
            if this.visible && !this.completed {
                send_future_event(FutureEvent::Cancelled {
                    future_id: this.future_id,
                    call_id: this.call_id,
                });
            }
        }
    }
}

impl<F: Future> InstrumentedFuture<F> {
    pub(crate) fn new(
        inner: F,
        location: &'static str,
        label: Option<String>,
        alloc_bridge: Option<Arc<AsyncAllocBridge>>,
        visible: bool,
        caller_scope: Option<&'static str>,
    ) -> Self {
        let _suspend = crate::lib_on::SuspendAllocTracking::new();

        // Per-call sampling decision: either every poll of this call is timed or
        // none are, so per-call poll stats are exact rather than extrapolated.
        let (future_id, call_id, timed) = if visible {
            ensure_futures_state();
            let (future_id, is_new) = get_or_create_future_id(location);
            let call_id = FUTURE_CALL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);

            if is_new {
                send_future_event(FutureEvent::Created {
                    future_id,
                    source: location,
                    display_label: label,
                });
            }

            send_future_event(FutureEvent::CallCreated { future_id, call_id });
            (
                future_id,
                call_id,
                crate::lib_on::sampling::futures_should_time(),
            )
        } else {
            (0, 0, false)
        };

        drop(_suspend);

        Self {
            inner,
            future_id,
            call_id,
            completed: false,
            visible,
            timed,
            alloc_bridge,
            caller_scope,
        }
    }
}

struct CallerScopeGuard(bool);

impl CallerScopeGuard {
    #[inline]
    fn enter(caller_scope: Option<&'static str>) -> Self {
        if let Some(scope) = caller_scope {
            crate::lib_on::caller_stack::push_caller(scope);
        }
        Self(caller_scope.is_some())
    }
}

impl Drop for CallerScopeGuard {
    #[inline]
    fn drop(&mut self) {
        if self.0 {
            crate::lib_on::caller_stack::pop_caller();
        }
    }
}

// Bounded on the wrapped future, so wrapping keeps `Debug` wherever it had it.
impl<F: Future + std::fmt::Debug> std::fmt::Debug for InstrumentedFuture<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstrumentedFuture")
            .field("inner", &self.inner)
            .field("id", &self.future_id)
            .finish_non_exhaustive()
    }
}

impl<F: Future> Future for InstrumentedFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let visible = *this.visible;

        // Don't instrument future unless visible, only collect alloc data
        if !visible {
            let (result, poll_alloc_bytes, poll_alloc_count) = measure_poll_alloc(|| {
                let _caller_scope = CallerScopeGuard::enter(*this.caller_scope);
                this.inner.poll(cx)
            });
            if let (Some(bytes), Some(count), Some(bridge)) = (
                poll_alloc_bytes,
                poll_alloc_count,
                this.alloc_bridge.as_ref(),
            ) {
                bridge.add(bytes, count);
            }
            if result.is_ready() {
                *this.completed = true;
            }
            return result;
        }

        let future_id = *this.future_id;
        let call_id = *this.call_id;

        let start = (*this.timed).then(Instant::now);
        let (result, poll_alloc_bytes, poll_alloc_count) = measure_poll_alloc(|| {
            let _caller_scope = CallerScopeGuard::enter(*this.caller_scope);
            this.inner.poll(cx)
        });
        let poll_duration_ns =
            start.map(|start| Instant::now().duration_since(start).as_nanos() as u64);
        if let (Some(bytes), Some(count), Some(bridge)) = (
            poll_alloc_bytes,
            poll_alloc_count,
            this.alloc_bridge.as_ref(),
        ) {
            bridge.add(bytes, count);
        }

        let poll_result = match &result {
            Poll::Pending => PollResult::Pending,
            Poll::Ready(_) => {
                *this.completed = true;
                PollResult::Ready
            }
        };

        {
            let _suspend = crate::lib_on::SuspendAllocTracking::new();
            send_future_event(FutureEvent::Polled {
                future_id,
                call_id,
                result: poll_result,
                poll_duration_ns,
                poll_alloc_bytes,
                poll_alloc_count,
            });

            if *this.completed {
                send_future_event(FutureEvent::Completed {
                    future_id,
                    call_id,
                    log_message: None,
                });
            }
        }

        result
    }
}

pin_project! {
    /// A wrapper around a future that tracks lifecycle events including the output value.
    ///
    /// Created via the `future!(expr, log = true)` macro, this wrapper tracks:
    /// - Creation
    /// - Each poll call with result (Pending/Ready with Debug output) and duration
    /// - Memory allocations per poll (when `hotpath-alloc` feature is enabled)
    /// - Drop (cancellation if not completed)
    ///
    /// This variant requires `Debug` on the output type to log the value.
    pub struct InstrumentedFutureLog<F: Future> {
        #[pin]
        inner: F,
        future_id: u32,
        call_id: u32,
        completed: bool,
        visible: bool,
        timed: bool,
        alloc_bridge: Option<Arc<AsyncAllocBridge>>,
        // See `InstrumentedFuture::caller_scope`.
        caller_scope: Option<&'static str>,
    }

    impl<F: Future> PinnedDrop for InstrumentedFutureLog<F> {
        fn drop(this: Pin<&mut Self>) {
            if this.visible && !this.completed {
                send_future_event(FutureEvent::Cancelled {
                    future_id: this.future_id,
                    call_id: this.call_id,
                });
            }
        }
    }
}

impl<F: Future> InstrumentedFutureLog<F> {
    pub(crate) fn new(
        inner: F,
        location: &'static str,
        label: Option<String>,
        alloc_bridge: Option<Arc<AsyncAllocBridge>>,
        visible: bool,
        caller_scope: Option<&'static str>,
    ) -> Self {
        let _suspend = crate::lib_on::SuspendAllocTracking::new();

        // Per-call sampling decision: either every poll of this call is timed or
        // none are, so per-call poll stats are exact rather than extrapolated.
        let (future_id, call_id, timed) = if visible {
            ensure_futures_state();
            let (future_id, is_new) = get_or_create_future_id(location);
            let call_id = FUTURE_CALL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);

            if is_new {
                send_future_event(FutureEvent::Created {
                    future_id,
                    source: location,
                    display_label: label,
                });
            }

            send_future_event(FutureEvent::CallCreated { future_id, call_id });
            (
                future_id,
                call_id,
                crate::lib_on::sampling::futures_should_time(),
            )
        } else {
            (0, 0, false)
        };

        drop(_suspend);

        Self {
            inner,
            future_id,
            call_id,
            completed: false,
            visible,
            timed,
            alloc_bridge,
            caller_scope,
        }
    }
}

impl<F: Future + std::fmt::Debug> std::fmt::Debug for InstrumentedFutureLog<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstrumentedFutureLog")
            .field("inner", &self.inner)
            .field("id", &self.future_id)
            .finish_non_exhaustive()
    }
}

impl<F: Future> Future for InstrumentedFutureLog<F>
where
    F::Output: std::fmt::Debug,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let visible = *this.visible;

        if !visible {
            let (result, poll_alloc_bytes, poll_alloc_count) = measure_poll_alloc(|| {
                let _caller_scope = CallerScopeGuard::enter(*this.caller_scope);
                this.inner.poll(cx)
            });
            if let (Some(bytes), Some(count), Some(bridge)) = (
                poll_alloc_bytes,
                poll_alloc_count,
                this.alloc_bridge.as_ref(),
            ) {
                bridge.add(bytes, count);
            }
            if result.is_ready() {
                *this.completed = true;
            }
            return result;
        }

        let future_id = *this.future_id;
        let call_id = *this.call_id;

        let start = (*this.timed).then(Instant::now);
        let (result, poll_alloc_bytes, poll_alloc_count) = measure_poll_alloc(|| {
            let _caller_scope = CallerScopeGuard::enter(*this.caller_scope);
            this.inner.poll(cx)
        });
        let poll_duration_ns =
            start.map(|start| Instant::now().duration_since(start).as_nanos() as u64);
        if let (Some(bytes), Some(count), Some(bridge)) = (
            poll_alloc_bytes,
            poll_alloc_count,
            this.alloc_bridge.as_ref(),
        ) {
            bridge.add(bytes, count);
        }

        let (poll_result, log_message) = match &result {
            Poll::Pending => (PollResult::Pending, None),
            Poll::Ready(value) => {
                *this.completed = true;
                (PollResult::Ready, Some(format_debug_truncated(value)))
            }
        };

        {
            let _suspend = crate::lib_on::SuspendAllocTracking::new();
            send_future_event(FutureEvent::Polled {
                future_id,
                call_id,
                result: poll_result,
                poll_duration_ns,
                poll_alloc_bytes,
                poll_alloc_count,
            });

            if *this.completed {
                send_future_event(FutureEvent::Completed {
                    future_id,
                    call_id,
                    log_message,
                });
            }
        }

        result
    }
}
