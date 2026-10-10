//! Endpoint-wrapping `futures_channel::mpsc` instrumentation for the `channel!` macro.
//!
//! Wraps the `Sender`/`Receiver` endpoints directly: sends and receives hit the real
//! channel and the only added cost is a non-blocking event emit.
//!
//! futures `mpsc` exposes no `len()` on the bounded endpoints, so `queue_len` is read
//! from a self-maintained `AtomicUsize`: incremented before each push (rolled back if the
//! push fails) and decremented after each receive. Counting before the push keeps the
//! counter non-negative - the channel's send->recv edge orders a producer's `+1` ahead of
//! the consumer's matching `-1`. Pushes never park (`poll_ready` does the waiting), so no
//! cancellation guard is needed. Exact for a single producer; with concurrent cloned
//! senders another sender's pre-push `+1` can land in a snapshot, so depth and high-water
//! marks may transiently read one higher per in-flight send.
//!
//! The inner channel carries `(msg_id, send_ts, T)`. Monotonic `msg_id` pairs a send with
//! its matching receive under multiple producers; `send_ts` is stamped before publishing,
//! so `send_ts <= recv_ts` always holds and the reported delay is non-negative. Both fields
//! are internal - the public API still uses `T`.
//!
//! `mpsc::TrySendError` has no public constructor, so `try_send` / `unbounded_send` return
//! this module's [`TrySendError`], which keeps the inner error whole and exposes the same
//! methods (`is_full`, `is_disconnected`, `into_inner`, `into_send_error`). `Sink` errors
//! are the plain `mpsc::SendError` and pass through unchanged.
//!
//! The wrapper rebuilds the inner channel, so the `channel!` expression must be
//! constructed inline; endpoints cloned before wrapping are orphaned. Bounded channels
//! expose no capacity accessor, so `capacity = N` is required and must match the
//! `channel(N)` argument.
//!
//! Receivers are single-consumer (not `Clone`) and emit `Closed` unconditionally on drop.
//!
//! Returns [`Sender`]/[`Receiver`]/[`UnboundedSender`]/[`UnboundedReceiver`], re-exported
//! as `hotpath::wrap::futures_channel::mpsc::*`.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_channel::mpsc;
use futures_channel::mpsc::{RecvError, SendError, TryRecvError};
use futures_core::stream::{FusedStream, Stream};
use futures_sink::Sink;

use crate::channels::wrapper::{Depth, LogFn, Payload, RecvSide, SendSide, WrapChannel};
use crate::channels::ChannelType;

/// Error returned by [`Sender::try_send`] and [`UnboundedSender::unbounded_send`].
///
/// Stands in for [`futures_channel::mpsc::TrySendError`], which cannot be rebuilt
/// around `T` once the payload is unwrapped; the API is the same.
pub struct TrySendError<T>(mpsc::TrySendError<Payload<T>>);

impl<T> TrySendError<T> {
    pub fn is_full(&self) -> bool {
        self.0.is_full()
    }

    pub fn is_disconnected(&self) -> bool {
        self.0.is_disconnected()
    }

    pub fn into_inner(self) -> T {
        self.0.into_inner().2
    }

    pub fn into_send_error(self) -> SendError {
        self.0.into_send_error()
    }
}

impl<T> std::fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> std::fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: 'static> std::error::Error for TrySendError<T> {}

/// Instrumented bounded [`futures_channel::mpsc::Sender`] wrapper.
pub struct Sender<T> {
    inner: mpsc::Sender<Payload<T>>,
    side: SendSide<T>,
    depth: Depth,
}

impl<T> Sender<T> {
    pub fn try_send(&mut self, msg: T) -> Result<(), TrySendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        let slot = self.depth.reserve();
        match self.inner.try_send(payload) {
            Ok(()) => {
                self.side.sent(stamp, slot.commit());
                Ok(())
            }
            Err(e) => Err(TrySendError(e)),
        }
    }

    pub fn start_send(&mut self, msg: T) -> Result<(), SendError> {
        self.try_send(msg).map_err(TrySendError::into_send_error)
    }

    pub fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        self.inner.poll_ready(cx)
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    pub fn close_channel(&mut self) {
        self.inner.close_channel();
    }

    pub fn disconnect(&mut self) {
        self.inner.disconnect();
    }

    pub fn same_receiver(&self, other: &Self) -> bool {
        self.inner.same_receiver(&other.inner)
    }

    pub fn is_connected_to(&self, receiver: &Receiver<T>) -> bool {
        self.inner.is_connected_to(&receiver.inner)
    }

    pub fn hash_receiver<H>(&self, hasher: &mut H)
    where
        H: std::hash::Hasher,
    {
        self.inner.hash_receiver(hasher);
    }
}

impl<T> Sink<T> for Sender<T> {
    type Error = SendError;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        self.inner.poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, msg: T) -> Result<(), SendError> {
        Sender::start_send(&mut self, msg)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            side: self.side.clone_handle(),
            depth: self.depth.clone(),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.side.drop_handle(|| self.depth.load());
    }
}

/// Instrumented bounded [`futures_channel::mpsc::Receiver`] wrapper (single consumer).
pub struct Receiver<T> {
    inner: mpsc::Receiver<Payload<T>>,
    side: RecvSide,
    depth: Depth,
}

impl<T> Receiver<T> {
    fn received(&self, (msg_id, send_ts, msg): Payload<T>) -> T {
        self.side.received(msg_id, send_ts, self.depth.release());
        msg
    }

    /// No `Closed` emit here: remaining messages still drain after `close()`, and
    /// the state machine treats `Closed` as terminal. The drop impl emits it.
    pub fn close(&mut self) {
        self.inner.close();
    }

    pub async fn recv(&mut self) -> Result<T, RecvError> {
        let payload = self.inner.recv().await?;
        Ok(self.received(payload))
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let payload = self.inner.try_recv()?;
        Ok(self.received(payload))
    }

    #[deprecated(note = "please use `try_recv` instead")]
    #[allow(deprecated)]
    pub fn try_next(&mut self) -> Result<Option<T>, TryRecvError> {
        Ok(self.inner.try_next()?.map(|payload| self.received(payload)))
    }
}

impl<T> Stream for Receiver<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        Pin::new(&mut self.inner)
            .poll_next(cx)
            .map(|item| item.map(|payload| self.received(payload)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<T> FusedStream for Receiver<T> {
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // Close first so a send racing this drop fails instead of landing after
        // the remaining count is read.
        self.inner.close();
        self.side.mark_closed(self.inner.size_hint().0);
    }
}

/// Instrumented [`futures_channel::mpsc::UnboundedSender`] wrapper.
pub struct UnboundedSender<T> {
    inner: mpsc::UnboundedSender<Payload<T>>,
    side: SendSide<T>,
    depth: Depth,
}

impl<T> UnboundedSender<T> {
    pub fn unbounded_send(&self, msg: T) -> Result<(), TrySendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        let slot = self.depth.reserve();
        match self.inner.unbounded_send(payload) {
            Ok(()) => {
                self.side.sent(stamp, slot.commit());
                Ok(())
            }
            Err(e) => Err(TrySendError(e)),
        }
    }

    pub fn start_send(&mut self, msg: T) -> Result<(), SendError> {
        self.unbounded_send(msg)
            .map_err(TrySendError::into_send_error)
    }

    pub fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        self.inner.poll_ready(cx)
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    pub fn close_channel(&self) {
        self.inner.close_channel();
    }

    pub fn disconnect(&mut self) {
        self.inner.disconnect();
    }

    pub fn same_receiver(&self, other: &Self) -> bool {
        self.inner.same_receiver(&other.inner)
    }

    pub fn is_connected_to(&self, receiver: &UnboundedReceiver<T>) -> bool {
        self.inner.is_connected_to(&receiver.inner)
    }

    pub fn hash_receiver<H>(&self, hasher: &mut H)
    where
        H: std::hash::Hasher,
    {
        self.inner.hash_receiver(hasher);
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl<T> Sink<T> for UnboundedSender<T> {
    type Error = SendError;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        self.inner.poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, msg: T) -> Result<(), SendError> {
        UnboundedSender::start_send(&mut self, msg)
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        Pin::new(&mut self.inner).poll_close(cx)
    }
}

impl<T> Sink<T> for &UnboundedSender<T> {
    type Error = SendError;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        self.inner.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, msg: T) -> Result<(), SendError> {
        self.unbounded_send(msg)
            .map_err(TrySendError::into_send_error)
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), SendError>> {
        self.close_channel();
        Poll::Ready(Ok(()))
    }
}

impl<T> Clone for UnboundedSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            side: self.side.clone_handle(),
            depth: self.depth.clone(),
        }
    }
}

impl<T> Drop for UnboundedSender<T> {
    fn drop(&mut self) {
        self.side.drop_handle(|| self.depth.load());
    }
}

/// Instrumented [`futures_channel::mpsc::UnboundedReceiver`] wrapper (single consumer).
pub struct UnboundedReceiver<T> {
    inner: mpsc::UnboundedReceiver<Payload<T>>,
    side: RecvSide,
    depth: Depth,
}

impl<T> UnboundedReceiver<T> {
    fn received(&self, (msg_id, send_ts, msg): Payload<T>) -> T {
        self.side.received(msg_id, send_ts, self.depth.release());
        msg
    }

    /// No `Closed` emit here: remaining messages still drain after `close()`, and
    /// the state machine treats `Closed` as terminal. The drop impl emits it.
    pub fn close(&mut self) {
        self.inner.close();
    }

    pub async fn recv(&mut self) -> Result<T, RecvError> {
        let payload = self.inner.recv().await?;
        Ok(self.received(payload))
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let payload = self.inner.try_recv()?;
        Ok(self.received(payload))
    }

    #[deprecated(note = "please use `try_recv` instead")]
    #[allow(deprecated)]
    pub fn try_next(&mut self) -> Result<Option<T>, TryRecvError> {
        Ok(self.inner.try_next()?.map(|payload| self.received(payload)))
    }
}

impl<T> Stream for UnboundedReceiver<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        Pin::new(&mut self.inner)
            .poll_next(cx)
            .map(|item| item.map(|payload| self.received(payload)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<T> FusedStream for UnboundedReceiver<T> {
    fn is_terminated(&self) -> bool {
        self.inner.is_terminated()
    }
}

impl<T> Drop for UnboundedReceiver<T> {
    fn drop(&mut self) {
        // Close first so a send racing this drop fails instead of landing after
        // the remaining count is read.
        self.inner.close();
        self.side.mark_closed(self.inner.size_hint().0);
    }
}

// futures' endpoints implement `Debug` for any `T`, so the wrappers delegate
// unconditionally too.
macro_rules! impl_debug_via_inner {
    ($($ty:ident),+) => {$(
        impl<T> std::fmt::Debug for $ty<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty))
                    .field("inner", &self.inner)
                    .finish_non_exhaustive()
            }
        }
    )+};
}

impl_debug_via_inner!(Sender, Receiver, UnboundedSender, UnboundedReceiver);

fn build_bounded<T>(
    source: &'static str,
    label: Option<String>,
    capacity: Option<usize>,
    log_fn: Option<LogFn<T>>,
    iter: bool,
) -> (Sender<T>, Receiver<T>) {
    let Some(capacity) = capacity else {
        panic!("Capacity is required for bounded futures channels, because they don't expose their capacity in a public API");
    };
    let side = SendSide::register(source, label, ChannelType::Bounded(capacity), log_fn, iter);
    // Unclamped: a bounded futures channel holds `capacity` plus one slot per sender.
    let depth = Depth::new(None);
    // Rebuild to carry the payload; the caller's channel is discarded (the wrapper
    // is inline-only).
    let (tx, rx) = mpsc::channel::<Payload<T>>(capacity);
    let receiver = Receiver {
        inner: rx,
        side: side.recv_side(),
        depth: depth.clone(),
    };
    let sender = Sender {
        inner: tx,
        side,
        depth,
    };
    (sender, receiver)
}

fn build_unbounded<T>(
    source: &'static str,
    label: Option<String>,
    log_fn: Option<LogFn<T>>,
    iter: bool,
) -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let side = SendSide::register(source, label, ChannelType::Unbounded, log_fn, iter);
    let depth = Depth::new(None);
    let (tx, rx) = mpsc::unbounded::<Payload<T>>();
    let receiver = UnboundedReceiver {
        inner: rx,
        side: side.recv_side(),
        depth: depth.clone(),
    };
    let sender = UnboundedSender {
        inner: tx,
        side,
        depth,
    };
    (sender, receiver)
}

impl<T: Send + 'static> WrapChannel for (mpsc::Sender<T>, mpsc::Receiver<T>) {
    type Msg = T;
    type Output = (Sender<T>, Receiver<T>);
    fn wrap(
        self,
        source: &'static str,
        label: Option<String>,
        capacity: Option<usize>,
        log_fn: Option<LogFn<T>>,
        iter: bool,
    ) -> Self::Output {
        build_bounded(source, label, capacity, log_fn, iter)
    }
}

impl<T: Send + 'static> WrapChannel for (mpsc::UnboundedSender<T>, mpsc::UnboundedReceiver<T>) {
    type Msg = T;
    type Output = (UnboundedSender<T>, UnboundedReceiver<T>);
    fn wrap(
        self,
        source: &'static str,
        label: Option<String>,
        _capacity: Option<usize>,
        log_fn: Option<LogFn<T>>,
        iter: bool,
    ) -> Self::Output {
        build_unbounded(source, label, log_fn, iter)
    }
}

#[cfg(test)]
mod tests {
    use crate::channels::wrapper::ftc_wrap::{
        build_bounded, build_unbounded, Receiver, Sender, UnboundedReceiver, UnboundedSender,
    };
    use futures_channel::mpsc::RecvError;
    use futures_core::stream::{FusedStream, Stream};
    use futures_util::{SinkExt, StreamExt};

    fn bounded<T: Send + 'static>(capacity: usize) -> (Sender<T>, Receiver<T>) {
        build_bounded::<T>("test", None, Some(capacity), None, false)
    }

    fn unbounded<T: Send + 'static>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
        build_unbounded::<T>("test", None, None, false)
    }

    #[test]
    fn send_only_payload_keeps_endpoints_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        // `Cell<u8>` is `Send` but not `Sync`, matching payloads like `Box<dyn Trait + Send>`.
        type P = std::cell::Cell<u8>;
        assert_send_sync::<Sender<P>>();
        assert_send_sync::<Receiver<P>>();
        assert_send_sync::<UnboundedSender<P>>();
        assert_send_sync::<UnboundedReceiver<P>>();
    }

    #[tokio::test]
    async fn sink_and_stream_roundtrip() {
        let (mut tx, mut rx) = bounded::<u32>(2);
        tx.send(1).await.unwrap();
        tx.send(2).await.unwrap();
        assert_eq!(rx.size_hint().0, 2);
        assert_eq!(rx.next().await, Some(1));
        assert_eq!(rx.next().await, Some(2));
        drop(tx);
        assert_eq!(rx.next().await, None);
        assert!(rx.is_terminated());

        let (mut tx, mut rx) = unbounded::<u32>();
        tx.send(7).await.unwrap();
        (&tx).send(8).await.unwrap();
        assert_eq!(tx.len(), 2);
        assert_eq!(rx.next().await, Some(7));
        assert_eq!(rx.next().await, Some(8));
        assert!(tx.is_empty());
    }

    #[tokio::test]
    async fn recv_returns_messages_then_error_when_closed() {
        let (mut tx, mut rx) = bounded::<u32>(1);
        tx.send(1).await.unwrap();
        assert_eq!(rx.recv().await, Ok(1));
        drop(tx);
        assert_eq!(rx.recv().await, Err(RecvError));

        let (tx, mut rx) = unbounded::<u32>();
        tx.unbounded_send(2).unwrap();
        assert_eq!(rx.recv().await, Ok(2));
        drop(tx);
        assert_eq!(rx.recv().await, Err(RecvError));
    }

    #[test]
    fn try_send_full_error_returns_value() {
        // A bounded futures channel holds `capacity + 1` per sender before it is full.
        let (mut tx, mut rx) = bounded::<u32>(0);
        tx.try_send(1).unwrap();
        let err = tx.try_send(2).unwrap_err();
        assert!(err.is_full());
        assert!(!err.is_disconnected());
        assert_eq!(err.into_inner(), 2);
        assert_eq!(rx.try_recv().unwrap(), 1);
        assert!(rx.try_recv().unwrap_err().is_empty());
    }

    #[test]
    fn send_after_receiver_drop_is_disconnected() {
        let (tx, rx) = unbounded::<u32>();
        drop(rx);
        assert!(tx.is_closed());
        let err = tx.unbounded_send(5).unwrap_err();
        assert!(err.is_disconnected());
        assert!(err.into_send_error().is_disconnected());

        let (mut tx, rx) = bounded::<u32>(1);
        drop(rx);
        assert!(tx.try_send(5).unwrap_err().is_disconnected());
        assert!(tx.start_send(6).unwrap_err().is_disconnected());
    }

    #[test]
    fn close_stops_sends_but_drains() {
        let (mut tx, mut rx) = bounded::<u32>(4);
        tx.try_send(1).unwrap();
        rx.close();
        assert!(tx.is_closed());
        assert!(tx.try_send(2).unwrap_err().is_disconnected());
        assert_eq!(rx.try_recv().unwrap(), 1);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn same_receiver_across_clones() {
        let (tx_a, rx_a) = bounded::<u32>(4);
        let (tx_b, _rx_b) = bounded::<u32>(4);
        assert!(tx_a.same_receiver(&tx_a.clone()));
        assert!(!tx_a.same_receiver(&tx_b));
        assert!(tx_a.is_connected_to(&rx_a));

        let (utx_a, urx_a) = unbounded::<u32>();
        let (utx_b, _urx_b) = unbounded::<u32>();
        assert!(utx_a.same_receiver(&utx_a.clone()));
        assert!(!utx_a.same_receiver(&utx_b));
        assert!(utx_a.is_connected_to(&urx_a));
    }
}
