//! Endpoint-wrapping async-channel instrumentation for the `channel!` macro.
//!
//! Wraps the `Sender`/`Receiver` endpoints directly, so send/recv hit the real channel.
//! `queue_len` is a snapshot taken right after each op via the exact
//! `async_channel::Sender::len`/`Receiver::len` - exact single-threaded, may skew under
//! concurrent endpoints.
//!
//! The inner channel carries `(msg_id, send_ts, T)`. Monotonic `msg_id` pairs a send with
//! its matching receive under multiple producers/consumers. `send_ts` is stamped before
//! publishing, so `send_ts <= recv_ts` always holds and the reported delay is non-negative;
//! for bounded channels it precedes the awaited send, so the delay includes backpressure
//! wait. Both fields are internal - the public API still uses `T`.
//!
//! async-channel is mpmc and both ends are `Clone`. Sender and receiver clones are counted,
//! and a `Closed` event fires when the last clone of either end is dropped (dropping all
//! receivers disconnects the channel, just like dropping all senders).
//!
//! The async (`send`/`recv`) and blocking (`send_blocking`/`recv_blocking`) APIs plus the
//! `try_*` variants are instrumented. `force_send`, `close`, and the `Receiver`'s `Stream`
//! impl are NOT wrapped - users on those paths silently lose instrumentation.
//!
//! The wrapper rebuilds the inner channel, so the `channel!` expression must be constructed
//! inline; endpoints cloned before wrapping are orphaned.
//!
//! Returns [`Sender`]/[`Receiver`], re-exported as `hotpath::wrap::async_channel::{Sender, Receiver}`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_channel::{
    Receiver as InnerReceiver, RecvError, SendError, Sender as InnerSender, TryRecvError,
    TrySendError,
};

use crate::channels::wrapper::{LogFn, Payload, RecvSide, SendSide, WrapChannel};
use crate::channels::ChannelType;

/// Instrumented async-channel [`async_channel::Sender`] wrapper.
///
/// Tracks every successful send and emits the exact channel length afterwards.
/// When the last clone is dropped, a `Closed` event is emitted.
pub struct Sender<T> {
    inner: InnerSender<Payload<T>>,
    side: SendSide<T>,
}

impl<T> Sender<T> {
    pub async fn send(&self, msg: T) -> Result<(), SendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        self.inner
            .send(payload)
            .await
            .map_err(|e| SendError(e.into_inner().2))?;
        self.side.sent(stamp, self.inner.len());
        Ok(())
    }

    pub fn send_blocking(&self, msg: T) -> Result<(), SendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        self.inner
            .send_blocking(payload)
            .map_err(|e| SendError(e.into_inner().2))?;
        self.side.sent(stamp, self.inner.len());
        Ok(())
    }

    pub fn try_send(&self, msg: T) -> Result<(), TrySendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        self.inner.try_send(payload).map_err(|e| match e {
            TrySendError::Full((_, _, msg)) => TrySendError::Full(msg),
            TrySendError::Closed((_, _, msg)) => TrySendError::Closed(msg),
        })?;
        self.side.sent(stamp, self.inner.len());
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    pub fn capacity(&self) -> Option<usize> {
        self.inner.capacity()
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            side: self.side.clone_handle(),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.side.drop_handle(|| self.inner.len());
    }
}

/// Instrumented async-channel [`async_channel::Receiver`] wrapper.
///
/// Tracks every successful receive and emits the exact channel length afterwards.
/// When the last clone is dropped, a `Closed` event is emitted, since dropping all
/// receivers disconnects the channel.
pub struct Receiver<T> {
    inner: InnerReceiver<Payload<T>>,
    side: RecvSide,
    receiver_count: Arc<AtomicUsize>,
}

impl<T> Receiver<T> {
    fn received(&self, (msg_id, send_ts, msg): Payload<T>) -> T {
        self.side.received(msg_id, send_ts, self.inner.len());
        msg
    }

    pub async fn recv(&self) -> Result<T, RecvError> {
        Ok(self.received(self.inner.recv().await?))
    }

    pub fn recv_blocking(&self) -> Result<T, RecvError> {
        Ok(self.received(self.inner.recv_blocking()?))
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        Ok(self.received(self.inner.try_recv()?))
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.inner.is_full()
    }

    pub fn capacity(&self) -> Option<usize> {
        self.inner.capacity()
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.receiver_count.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: self.inner.clone(),
            side: self.side.clone(),
            receiver_count: Arc::clone(&self.receiver_count),
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        if self.receiver_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.side.mark_closed(self.inner.len());
        }
    }
}

fn build<T>(
    inner: (InnerSender<T>, InnerReceiver<T>),
    source: &'static str,
    label: Option<String>,
    log_fn: Option<LogFn<T>>,
    iter: bool,
) -> (Sender<T>, Receiver<T>) {
    // Rebuild the inner channel to carry the payload. The caller's original channel
    // is discarded (the wrapper is inline-only, see module docs); only its
    // kind/capacity is copied.
    let (channel_type, (tx, rx)) = match inner.0.capacity() {
        Some(capacity) => (
            ChannelType::Bounded(capacity),
            async_channel::bounded::<Payload<T>>(capacity),
        ),
        None => (
            ChannelType::Unbounded,
            async_channel::unbounded::<Payload<T>>(),
        ),
    };
    let side = SendSide::register(source, label, channel_type, log_fn, iter);
    let receiver = Receiver {
        inner: rx,
        side: side.recv_side(),
        receiver_count: Arc::new(AtomicUsize::new(1)),
    };
    (Sender { inner: tx, side }, receiver)
}

impl<T: Send + 'static> WrapChannel for (InnerSender<T>, InnerReceiver<T>) {
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
        build(self, source, label, log_fn, iter)
    }
}
