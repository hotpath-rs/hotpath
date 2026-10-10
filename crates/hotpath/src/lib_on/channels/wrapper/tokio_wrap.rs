//! Endpoint-wrapping Tokio `mpsc` channel instrumentation for the `channel!` macro.
//!
//! Wraps the `Sender`/`Receiver` endpoints directly: send/recv hit the real channel and
//! the only added cost is a non-blocking event emit.
//!
//! Tokio `mpsc` exposes no cheap exact `len()` on the sender side, so `queue_len` is read
//! from a self-maintained `AtomicUsize`: incremented before each publish (rolled back if
//! the send fails) and decremented after each receive. Counting before the publish keeps
//! the counter non-negative - the channel's send->recv edge orders a producer's `+1` ahead
//! of the consumer's matching `-1`. A drop guard rolls the `+1` back unless the send
//! completes successfully, so a bounded async `send` cancelled while parked on a full
//! channel leaves no residue - the depth tracks successful sends exactly (it also feeds
//! the `Abandoned` count on teardown, which must not inherit phantom messages).
//!
//! The inner channel carries `(msg_id, send_ts, T)`. Monotonic `msg_id` pairs a send with
//! its matching receive under multiple producers; `send_ts` is stamped before publishing,
//! so `send_ts <= recv_ts` always holds and the reported delay is non-negative. For
//! bounded channels it precedes the (awaited) send, so the delay includes backpressure
//! wait. Both fields are internal - the public API still uses `T`.
//!
//! The wrapper rebuilds the inner channel, so the `channel!` expression must be
//! constructed inline; endpoints cloned before wrapping are orphaned. Bounded capacity is
//! recovered from `Sender::max_capacity()`, so no `capacity = N` argument is needed.
//!
//! Tokio `Receiver` is single-consumer (not `Clone`), so there is exactly one receiver and
//! it emits `Closed` unconditionally on drop.
//!
//! Returns [`Sender`]/[`Receiver`]/[`UnboundedSender`]/[`UnboundedReceiver`] (plus
//! [`WeakSender`]/[`WeakUnboundedSender`] via `downgrade`), re-exported as
//! `hotpath::wrap::tokio::sync::mpsc::*`.

use std::task::{Context, Poll};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::sync::mpsc::error::{SendError, SendTimeoutError, TryRecvError, TrySendError};

use crate::channels::wrapper::{
    Depth, LogFn, Payload, RecvSide, SendSide, WeakSendSide, WrapChannel,
};
use crate::channels::ChannelType;

/// Instrumented bounded [`tokio::sync::mpsc::Sender`] wrapper.
pub struct Sender<T> {
    inner: mpsc::Sender<Payload<T>>,
    side: SendSide<T>,
    depth: Depth,
}

impl<T> Sender<T> {
    pub async fn send(&self, msg: T) -> Result<(), SendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        // Held across the await: a send cancelled while parked rolls the depth back.
        let slot = self.depth.reserve();
        match self.inner.send(payload).await {
            Ok(()) => {
                self.side.sent(stamp, slot.commit());
                Ok(())
            }
            Err(SendError((_, _, msg))) => Err(SendError(msg)),
        }
    }

    pub fn try_send(&self, msg: T) -> Result<(), TrySendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        let slot = self.depth.reserve();
        match self.inner.try_send(payload) {
            Ok(()) => {
                self.side.sent(stamp, slot.commit());
                Ok(())
            }
            Err(TrySendError::Full((_, _, msg))) => Err(TrySendError::Full(msg)),
            Err(TrySendError::Closed((_, _, msg))) => Err(TrySendError::Closed(msg)),
        }
    }

    pub async fn send_timeout(&self, msg: T, timeout: Duration) -> Result<(), SendTimeoutError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        // Held across the await: a send cancelled while parked rolls the depth back.
        let slot = self.depth.reserve();
        match self.inner.send_timeout(payload, timeout).await {
            Ok(()) => {
                self.side.sent(stamp, slot.commit());
                Ok(())
            }
            Err(SendTimeoutError::Timeout((_, _, msg))) => Err(SendTimeoutError::Timeout(msg)),
            Err(SendTimeoutError::Closed((_, _, msg))) => Err(SendTimeoutError::Closed(msg)),
        }
    }

    /// Event emission is a sync crossbeam send, so it is safe off-runtime; like
    /// tokio's `blocking_send` this panics when called from an async context.
    pub fn blocking_send(&self, msg: T) -> Result<(), SendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        let slot = self.depth.reserve();
        match self.inner.blocking_send(payload) {
            Ok(()) => {
                self.side.sent(stamp, slot.commit());
                Ok(())
            }
            Err(SendError((_, _, msg))) => Err(SendError(msg)),
        }
    }

    pub async fn closed(&self) {
        self.inner.closed().await
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Wrapper clones share one inner channel, so delegating is exact.
    pub fn same_channel(&self, other: &Self) -> bool {
        self.inner.same_channel(&other.inner)
    }

    /// No `sender_count` bump: weak handles don't hold the channel open, and
    /// dropping one never emits `Closed`.
    pub fn downgrade(&self) -> WeakSender<T> {
        WeakSender {
            inner: self.inner.downgrade(),
            side: self.side.downgrade(),
            depth: self.depth.clone(),
        }
    }

    /// Every wrapper `Sender` holds exactly one inner sender, so tokio's counts
    /// equal the wrapper counts.
    pub fn strong_count(&self) -> usize {
        self.inner.strong_count()
    }

    pub fn weak_count(&self) -> usize {
        self.inner.weak_count()
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    pub fn max_capacity(&self) -> usize {
        self.inner.max_capacity()
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

/// Weak handle to an instrumented bounded sender, returned by [`Sender::downgrade`].
/// Holds no `sender_count` slot and has no `Drop` impl - weak handles don't keep the
/// channel open, so dropping them never emits `Closed`.
pub struct WeakSender<T> {
    inner: mpsc::WeakSender<Payload<T>>,
    side: WeakSendSide<T>,
    depth: Depth,
}

impl<T> WeakSender<T> {
    /// Fails once the last strong `Sender` has dropped: between its
    /// `sender_count` decrement (which emits `Closed`) and its inner sender
    /// actually dropping, tokio's `upgrade` can still succeed, so a plain
    /// delegate would resurrect a channel already marked terminal-closed.
    /// `WeakSendSide::upgrade` refuses the upgrade instead.
    pub fn upgrade(&self) -> Option<Sender<T>> {
        let inner = self.inner.upgrade()?;
        Some(Sender {
            inner,
            side: self.side.upgrade()?,
            depth: self.depth.clone(),
        })
    }

    pub fn strong_count(&self) -> usize {
        self.inner.strong_count()
    }

    pub fn weak_count(&self) -> usize {
        self.inner.weak_count()
    }
}

impl<T> Clone for WeakSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            side: self.side.clone(),
            depth: self.depth.clone(),
        }
    }
}

/// Instrumented bounded [`tokio::sync::mpsc::Receiver`] wrapper (single consumer).
pub struct Receiver<T> {
    inner: mpsc::Receiver<Payload<T>>,
    side: RecvSide,
    depth: Depth,
    /// Scratch buffer for the `recv_many` variants: tokio fills a payload-typed
    /// buffer, so messages land here first and are restamped into the caller's
    /// buffer. Reused across calls to keep the steady state allocation-free.
    poll_buf: Vec<Payload<T>>,
}

impl<T> Receiver<T> {
    fn received(&self, (msg_id, send_ts, msg): Payload<T>) -> T {
        self.side.received(msg_id, send_ts, self.depth.release());
        msg
    }

    /// Restamps `poll_buf` into `buffer`, one receive event per message, so msg-id
    /// pairing, delay histograms, and queue-depth accounting stay exact.
    fn flush_poll_buf(&mut self, buffer: &mut Vec<T>) {
        let mut payloads = std::mem::take(&mut self.poll_buf);
        buffer.reserve(payloads.len());
        for payload in payloads.drain(..) {
            buffer.push(self.received(payload));
        }
        self.poll_buf = payloads;
    }

    pub async fn recv(&mut self) -> Option<T> {
        let payload = self.inner.recv().await?;
        Some(self.received(payload))
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let payload = self.inner.try_recv()?;
        Ok(self.received(payload))
    }

    pub async fn recv_many(&mut self, buffer: &mut Vec<T>, limit: usize) -> usize {
        let n = self.inner.recv_many(&mut self.poll_buf, limit).await;
        self.flush_poll_buf(buffer);
        n
    }

    /// Event emission is a sync crossbeam send, so it is safe off-runtime; like
    /// tokio's `blocking_recv` this panics when called from an async context.
    pub fn blocking_recv(&mut self) -> Option<T> {
        let payload = self.inner.blocking_recv()?;
        Some(self.received(payload))
    }

    pub fn blocking_recv_many(&mut self, buffer: &mut Vec<T>, limit: usize) -> usize {
        let n = self.inner.blocking_recv_many(&mut self.poll_buf, limit);
        self.flush_poll_buf(buffer);
        n
    }

    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.inner
            .poll_recv(cx)
            .map(|item| item.map(|payload| self.received(payload)))
    }

    /// On `Pending` the scratch buffer is untouched, so no state leaks across polls.
    pub fn poll_recv_many(
        &mut self,
        cx: &mut Context<'_>,
        buffer: &mut Vec<T>,
        limit: usize,
    ) -> Poll<usize> {
        match self.inner.poll_recv_many(cx, &mut self.poll_buf, limit) {
            Poll::Ready(n) => {
                self.flush_poll_buf(buffer);
                Poll::Ready(n)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    /// No `Closed` emit here: remaining messages still drain after `close()`, and
    /// the state machine treats `Closed` as terminal, so an early emit would mark
    /// the channel closed while receive events keep arriving. The drop impl emits it.
    pub fn close(&mut self) {
        self.inner.close();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Delegates to the inner channel rather than the `depth` atomic - `depth`
    /// over-counts by the number of cancelled bounded sends, while the inner
    /// payload count equals the message count exactly.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn sender_strong_count(&self) -> usize {
        self.inner.sender_strong_count()
    }

    pub fn sender_weak_count(&self) -> usize {
        self.inner.sender_weak_count()
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.side.mark_closed(self.inner.len());
    }
}

// SAFETY: restores tokio's `Receiver<T>: Sync` bound of `T: Send`, which the
// auto impl loses to `poll_buf` (`Vec<T>` demands `T: Sync`). Sound because
// `poll_buf` is only touched through `&mut self` - no `&self` method can reach
// a `T` in it.
unsafe impl<T: Send> Sync for Receiver<T> {}

/// Instrumented [`tokio::sync::mpsc::UnboundedSender`] wrapper.
pub struct UnboundedSender<T> {
    inner: mpsc::UnboundedSender<Payload<T>>,
    side: SendSide<T>,
    depth: Depth,
}

impl<T> UnboundedSender<T> {
    pub fn send(&self, msg: T) -> Result<(), SendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        let slot = self.depth.reserve();
        match self.inner.send(payload) {
            Ok(()) => {
                self.side.sent(stamp, slot.commit());
                Ok(())
            }
            Err(SendError((_, _, msg))) => Err(SendError(msg)),
        }
    }

    pub async fn closed(&self) {
        self.inner.closed().await
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Wrapper clones share one inner channel, so delegating is exact.
    pub fn same_channel(&self, other: &Self) -> bool {
        self.inner.same_channel(&other.inner)
    }

    /// No `sender_count` bump: weak handles don't hold the channel open, and
    /// dropping one never emits `Closed`.
    pub fn downgrade(&self) -> WeakUnboundedSender<T> {
        WeakUnboundedSender {
            inner: self.inner.downgrade(),
            side: self.side.downgrade(),
            depth: self.depth.clone(),
        }
    }

    /// Every wrapper `UnboundedSender` holds exactly one inner sender, so tokio's
    /// counts equal the wrapper counts.
    pub fn strong_count(&self) -> usize {
        self.inner.strong_count()
    }

    pub fn weak_count(&self) -> usize {
        self.inner.weak_count()
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

/// Weak handle to an instrumented unbounded sender, returned by
/// [`UnboundedSender::downgrade`]. Holds no `sender_count` slot and has no `Drop`
/// impl - weak handles don't keep the channel open, so dropping them never emits
/// `Closed`.
pub struct WeakUnboundedSender<T> {
    inner: mpsc::WeakUnboundedSender<Payload<T>>,
    side: WeakSendSide<T>,
    depth: Depth,
}

impl<T> WeakUnboundedSender<T> {
    /// Fails once the last strong `UnboundedSender` has dropped: between its
    /// `sender_count` decrement (which emits `Closed`) and its inner sender
    /// actually dropping, tokio's `upgrade` can still succeed, so a plain
    /// delegate would resurrect a channel already marked terminal-closed.
    /// `WeakSendSide::upgrade` refuses the upgrade instead.
    pub fn upgrade(&self) -> Option<UnboundedSender<T>> {
        let inner = self.inner.upgrade()?;
        Some(UnboundedSender {
            inner,
            side: self.side.upgrade()?,
            depth: self.depth.clone(),
        })
    }

    pub fn strong_count(&self) -> usize {
        self.inner.strong_count()
    }

    pub fn weak_count(&self) -> usize {
        self.inner.weak_count()
    }
}

impl<T> Clone for WeakUnboundedSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            side: self.side.clone(),
            depth: self.depth.clone(),
        }
    }
}

/// Instrumented [`tokio::sync::mpsc::UnboundedReceiver`] wrapper (single consumer).
pub struct UnboundedReceiver<T> {
    inner: mpsc::UnboundedReceiver<Payload<T>>,
    side: RecvSide,
    depth: Depth,
    /// Scratch buffer for the `recv_many` variants: tokio fills a payload-typed
    /// buffer, so messages land here first and are restamped into the caller's
    /// buffer. Reused across calls to keep the steady state allocation-free.
    poll_buf: Vec<Payload<T>>,
}

impl<T> UnboundedReceiver<T> {
    fn received(&self, (msg_id, send_ts, msg): Payload<T>) -> T {
        self.side.received(msg_id, send_ts, self.depth.release());
        msg
    }

    /// Restamps `poll_buf` into `buffer`, one receive event per message, so msg-id
    /// pairing, delay histograms, and queue-depth accounting stay exact.
    fn flush_poll_buf(&mut self, buffer: &mut Vec<T>) {
        let mut payloads = std::mem::take(&mut self.poll_buf);
        buffer.reserve(payloads.len());
        for payload in payloads.drain(..) {
            buffer.push(self.received(payload));
        }
        self.poll_buf = payloads;
    }

    pub async fn recv(&mut self) -> Option<T> {
        let payload = self.inner.recv().await?;
        Some(self.received(payload))
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let payload = self.inner.try_recv()?;
        Ok(self.received(payload))
    }

    pub async fn recv_many(&mut self, buffer: &mut Vec<T>, limit: usize) -> usize {
        let n = self.inner.recv_many(&mut self.poll_buf, limit).await;
        self.flush_poll_buf(buffer);
        n
    }

    /// Event emission is a sync crossbeam send, so it is safe off-runtime; like
    /// tokio's `blocking_recv` this panics when called from an async context.
    pub fn blocking_recv(&mut self) -> Option<T> {
        let payload = self.inner.blocking_recv()?;
        Some(self.received(payload))
    }

    pub fn blocking_recv_many(&mut self, buffer: &mut Vec<T>, limit: usize) -> usize {
        let n = self.inner.blocking_recv_many(&mut self.poll_buf, limit);
        self.flush_poll_buf(buffer);
        n
    }

    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.inner
            .poll_recv(cx)
            .map(|item| item.map(|payload| self.received(payload)))
    }

    /// On `Pending` the scratch buffer is untouched, so no state leaks across polls.
    pub fn poll_recv_many(
        &mut self,
        cx: &mut Context<'_>,
        buffer: &mut Vec<T>,
        limit: usize,
    ) -> Poll<usize> {
        match self.inner.poll_recv_many(cx, &mut self.poll_buf, limit) {
            Poll::Ready(n) => {
                self.flush_poll_buf(buffer);
                Poll::Ready(n)
            }
            Poll::Pending => Poll::Pending,
        }
    }

    /// No `Closed` emit here: remaining messages still drain after `close()`, and
    /// the state machine treats `Closed` as terminal, so an early emit would mark
    /// the channel closed while receive events keep arriving. The drop impl emits it.
    pub fn close(&mut self) {
        self.inner.close();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Delegates to the inner channel rather than the `depth` atomic - `depth`
    /// over-counts by the number of cancelled sends, while the inner payload count
    /// equals the message count exactly.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn sender_strong_count(&self) -> usize {
        self.inner.sender_strong_count()
    }

    pub fn sender_weak_count(&self) -> usize {
        self.inner.sender_weak_count()
    }
}

impl<T> Drop for UnboundedReceiver<T> {
    fn drop(&mut self) {
        self.side.mark_closed(self.inner.len());
    }
}

// SAFETY: restores tokio's `UnboundedReceiver<T>: Sync` bound of `T: Send`,
// which the auto impl loses to `poll_buf` (`Vec<T>` demands `T: Sync`). Sound
// because `poll_buf` is only touched through `&mut self` - no `&self` method
// can reach a `T` in it.
unsafe impl<T: Send> Sync for UnboundedReceiver<T> {}

// Tokio's endpoints implement `Debug` for any `T`, so the wrappers delegate
// unconditionally too.
macro_rules! impl_debug_via_inner {
    ($($ty:ident),+) => {$(
        impl<T> std::fmt::Debug for $ty<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty))
                    .field("inner", &self.inner)
                    .field("id", &self.side.id())
                    .finish_non_exhaustive()
            }
        }
    )+};
}

impl_debug_via_inner!(
    Sender,
    Receiver,
    UnboundedSender,
    UnboundedReceiver,
    WeakSender,
    WeakUnboundedSender
);

fn build_bounded<T>(
    inner: (mpsc::Sender<T>, mpsc::Receiver<T>),
    source: &'static str,
    label: Option<String>,
    log_fn: Option<LogFn<T>>,
    iter: bool,
) -> (Sender<T>, Receiver<T>) {
    let capacity = inner.0.max_capacity();
    let side = SendSide::register(source, label, ChannelType::Bounded(capacity), log_fn, iter);
    let depth = Depth::new(Some(capacity));
    // Rebuild to carry the payload; the caller's channel is discarded (the wrapper
    // is inline-only), only its capacity is copied.
    let (tx, rx) = mpsc::channel::<Payload<T>>(capacity);
    let receiver = Receiver {
        inner: rx,
        side: side.recv_side(),
        depth: depth.clone(),
        poll_buf: Vec::new(),
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
    let (tx, rx) = mpsc::unbounded_channel::<Payload<T>>();
    let receiver = UnboundedReceiver {
        inner: rx,
        side: side.recv_side(),
        depth: depth.clone(),
        poll_buf: Vec::new(),
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
        _capacity: Option<usize>,
        log_fn: Option<LogFn<T>>,
        iter: bool,
    ) -> Self::Output {
        build_bounded(self, source, label, log_fn, iter)
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
    use crate::lib_on::channels::wrapper::tokio_wrap::*;
    use std::future::poll_fn;

    fn bounded<T: Send + 'static>(capacity: usize) -> (Sender<T>, Receiver<T>) {
        build_bounded(mpsc::channel::<T>(capacity), "test", None, None, false)
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
        assert_send_sync::<WeakSender<P>>();
        assert_send_sync::<WeakUnboundedSender<P>>();
    }

    #[test]
    fn weak_sender_upgrade_lifecycle() {
        let (tx, rx) = bounded::<u32>(4);
        let weak = tx.downgrade();
        assert_eq!(tx.strong_count(), 1);
        assert_eq!(tx.weak_count(), 1);

        let tx2 = weak.upgrade().expect("upgrade with strong sender alive");
        assert_eq!(tx.strong_count(), 2);
        assert_eq!(rx.sender_strong_count(), tx.strong_count());
        assert_eq!(rx.sender_weak_count(), tx.weak_count());

        drop(tx);
        drop(tx2);
        assert!(weak.upgrade().is_none());
        assert_eq!(weak.strong_count(), 0);
    }

    #[test]
    fn weak_unbounded_sender_upgrade_lifecycle() {
        let (tx, rx) = unbounded::<u32>();
        let weak = tx.downgrade();
        let tx2 = weak.upgrade().expect("upgrade with strong sender alive");
        assert_eq!(rx.sender_strong_count(), 2);
        assert_eq!(rx.sender_weak_count(), 1);

        drop(tx);
        drop(tx2);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn same_channel_across_clones() {
        let (tx_a, _rx_a) = bounded::<u32>(4);
        let (tx_b, _rx_b) = bounded::<u32>(4);
        assert!(tx_a.same_channel(&tx_a.clone()));
        assert!(!tx_a.same_channel(&tx_b));

        let (utx_a, _urx_a) = unbounded::<u32>();
        let (utx_b, _urx_b) = unbounded::<u32>();
        assert!(utx_a.same_channel(&utx_a.clone()));
        assert!(!utx_a.same_channel(&utx_b));
    }

    #[test]
    fn len_tracks_inner_channel() {
        let (tx, mut rx) = bounded::<u32>(4);
        assert!(rx.is_empty());
        tx.try_send(1).unwrap();
        tx.try_send(2).unwrap();
        assert_eq!(rx.len(), 2);
        rx.try_recv().unwrap();
        assert_eq!(rx.len(), 1);
        rx.try_recv().unwrap();
        assert!(rx.is_empty());
    }

    #[test]
    fn close_stops_sends_but_drains() {
        let (tx, mut rx) = bounded::<u32>(4);
        tx.try_send(1).unwrap();
        rx.close();
        assert!(rx.is_closed());
        assert!(matches!(tx.try_send(2), Err(TrySendError::Closed(2))));
        assert_eq!(rx.try_recv(), Ok(1));
    }

    #[test]
    fn blocking_send_recv_off_runtime() {
        let (tx, mut rx) = bounded::<u32>(4);
        let producer = std::thread::spawn(move || {
            for i in 0..25 {
                tx.blocking_send(i).unwrap();
            }
        });
        let consumer = std::thread::spawn(move || {
            let mut buf = Vec::new();
            while let Some(v) = rx.blocking_recv() {
                buf.push(v);
                if rx.blocking_recv_many(&mut buf, 8) == 0 {
                    break;
                }
            }
            buf
        });
        producer.join().unwrap();
        let buf = consumer.join().unwrap();
        assert_eq!(buf, (0..25).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn cancelled_send_rolls_back_depth() {
        use std::future::Future;

        let (tx, mut rx) = bounded::<u32>(1);
        tx.send(0).await.unwrap();
        assert_eq!(tx.depth.load(), 1);

        // Park a send on the full channel, then cancel it by dropping the
        // future: the pre-send depth increment must be rolled back so it
        // cannot leak into queue_len or the teardown Abandoned count.
        {
            let mut fut = Box::pin(tx.send(1));
            poll_fn(|cx| {
                assert!(fut.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        assert_eq!(tx.depth.load(), 1, "cancelled send must not leak depth");

        assert_eq!(rx.recv().await, Some(0));
        assert_eq!(tx.depth.load(), 0);
    }

    #[tokio::test]
    async fn send_timeout_rolls_back_depth() {
        let (tx, mut rx) = bounded::<u32>(2);
        tx.send(0).await.unwrap();
        tx.send(1).await.unwrap();
        let err = tx.send_timeout(2, Duration::from_millis(10)).await;
        assert!(matches!(err, Err(SendTimeoutError::Timeout(2))));
        assert_eq!(rx.len(), 2);
        assert_eq!(rx.recv().await, Some(0));
        tx.send_timeout(3, Duration::from_millis(10)).await.unwrap();
    }

    #[tokio::test]
    async fn poll_recv_many_pending_then_ready() {
        let (tx, mut rx) = bounded::<u32>(8);
        let mut buf = Vec::new();

        let was_pending =
            poll_fn(|cx| Poll::Ready(rx.poll_recv_many(cx, &mut buf, 4).is_pending())).await;
        assert!(was_pending);
        assert!(buf.is_empty());

        for i in 0..6 {
            tx.send(i).await.unwrap();
        }
        let n = poll_fn(|cx| rx.poll_recv_many(cx, &mut buf, 4)).await;
        assert_eq!(n, 4);
        let n = poll_fn(|cx| rx.poll_recv_many(cx, &mut buf, 4)).await;
        assert_eq!(n, 2);
        assert_eq!(buf, (0..6).collect::<Vec<_>>());

        let was_pending = poll_fn(|cx| Poll::Ready(rx.poll_recv(cx).is_pending())).await;
        assert!(was_pending);
        drop(tx);
        let closed = poll_fn(|cx| rx.poll_recv(cx)).await;
        assert_eq!(closed, None);
    }
}
