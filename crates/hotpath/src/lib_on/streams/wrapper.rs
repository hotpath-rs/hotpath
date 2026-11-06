use crate::output_on::format_debug_truncated;
use crate::streams::{register_stream, send_stream_event, StreamEvent};
use futures_core::Stream;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::instant::Instant;

pin_project! {
    /// A wrapper around a `Stream` that tracks item-yield events.
    ///
    /// Created via the `stream!` macro, this wrapper tracks:
    /// - Creation (stream type and item size)
    /// - Each item yield with timestamp
    /// - Stream completion
    ///
    /// This variant does NOT require `Debug` on the item type.
    /// Use `InstrumentedStreamLog` (via `stream!(expr, log = true)`) to log each yielded item.
    pub struct InstrumentedStream<S> {
        #[pin]
        inner: S,
        id: u32,
        completed: bool,
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl<S> InstrumentedStream<S> {
    pub(crate) fn new(stream: S, source: &'static str, label: Option<String>, iter: bool) -> Self
    where
        S: Stream,
    {
        let id = register_stream::<S::Item>(source, label, iter);
        Self {
            inner: stream,
            id,
            completed: false,
        }
    }
}

// Bounded on the wrapped stream, so wrapping keeps `Debug` wherever it had it.
impl<S: std::fmt::Debug> std::fmt::Debug for InstrumentedStream<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstrumentedStream")
            .field("inner", &self.inner)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl<S: Stream> Stream for InstrumentedStream<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        match this.inner.poll_next(cx) {
            Poll::Ready(Some(item)) => {
                send_stream_event(StreamEvent::Yielded {
                    id: *this.id,
                    log: None,
                    timestamp: Instant::now(),
                });
                Poll::Ready(Some(item))
            }
            Poll::Ready(None) => {
                // A fused stream can keep returning `Ready(None)`; `Completed`
                // must count each instance once or the entry's
                // `closed_instances` overshoots and closes the entry early.
                if !*this.completed {
                    *this.completed = true;
                    send_stream_event(StreamEvent::Completed { id: *this.id });
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

pin_project! {
    /// A wrapper around a `Stream` that tracks item-yield events including the item value.
    ///
    /// Created via the `stream!(expr, log = true)` macro, this wrapper tracks:
    /// - Creation (stream type and item size)
    /// - Each item yield with timestamp and `Debug` representation
    /// - Stream completion
    ///
    /// This variant requires `Debug` on the item type to log each yielded value.
    pub struct InstrumentedStreamLog<S> {
        #[pin]
        inner: S,
        id: u32,
        completed: bool,
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl<S> InstrumentedStreamLog<S> {
    pub(crate) fn new(stream: S, source: &'static str, label: Option<String>, iter: bool) -> Self
    where
        S: Stream,
    {
        let id = register_stream::<S::Item>(source, label, iter);
        Self {
            inner: stream,
            id,
            completed: false,
        }
    }
}

impl<S: std::fmt::Debug> std::fmt::Debug for InstrumentedStreamLog<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstrumentedStreamLog")
            .field("inner", &self.inner)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl<S: Stream> Stream for InstrumentedStreamLog<S>
where
    S::Item: std::fmt::Debug,
{
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        match this.inner.poll_next(cx) {
            Poll::Ready(Some(item)) => {
                let log_msg = format_debug_truncated(&item);
                send_stream_event(StreamEvent::Yielded {
                    id: *this.id,
                    log: Some(log_msg),
                    timestamp: Instant::now(),
                });
                Poll::Ready(Some(item))
            }
            Poll::Ready(None) => {
                // A fused stream can keep returning `Ready(None)`; `Completed`
                // must count each instance once or the entry's
                // `closed_instances` overshoots and closes the entry early.
                if !*this.completed {
                    *this.completed = true;
                    send_stream_event(StreamEvent::Completed { id: *this.id });
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
