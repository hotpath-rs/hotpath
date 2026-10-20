//! Endpoint-wrapping flume channel instrumentation for the `channel!` macro.
//!
//! Wraps the `Sender`/`Receiver` endpoints directly, so send/recv hit the real channel.
//! `queue_len` is a snapshot taken right after each op via the
//! exact `flume::Sender::len`/`Receiver::len` - exact single-threaded, may skew under
//! concurrent endpoints.
//!
//! The inner channel carries `(msg_id, send_ts, T)`. Monotonic `msg_id` pairs a send
//! with its matching receive under multiple producers/consumers. `send_ts` is stamped
//! before publishing, so `send_ts <= recv_ts` always holds and the reported delay is
//! non-negative; for bounded channels it precedes the blocking send, so the delay
//! includes backpressure wait. Both fields are internal - the public API still uses `T`.
//!
//! flume is mpmc and both ends are `Clone`. Sender and receiver clones are counted, and
//! a `Closed` event fires when the last clone of either end is dropped (dropping all
//! receivers disconnects the channel, just like dropping all senders).
//!
//! The sync (`send`/`recv` + timeouts) and async (`send_async`/`recv_async`) APIs are
//! instrumented. The `Stream`/`Sink` adapters (`rx.stream()`, `rx.into_stream()`, the
//! `Sender` `Sink` impl) and `send_deadline`/`recv_deadline`/`drain` are NOT wrapped -
//! users on those paths silently lose instrumentation.
//!
//! The wrapper rebuilds the inner channel, so the `channel!` expression must be
//! constructed inline; endpoints cloned before wrapping are orphaned.
//!
//! Returns [`Sender`]/[`Receiver`], re-exported as `hotpath::wrap::flume::{Sender, Receiver}`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use flume::{
    Receiver as InnerReceiver, RecvError, RecvTimeoutError, SendError, SendTimeoutError,
    Sender as InnerSender, TryRecvError, TrySendError,
};

use crate::channels::wrapper::{LogFn, Payload, RecvSide, SendSide, WrapChannel};
use crate::channels::ChannelType;

/// Instrumented flume [`flume::Sender`] wrapper.
///
/// Tracks every successful send and emits the exact channel length afterwards.
/// When the last clone is dropped, a `Closed` event is emitted.
pub struct Sender<T> {
    inner: InnerSender<Payload<T>>,
    side: SendSide<T>,
}

impl<T> Sender<T> {
    pub fn send(&self, msg: T) -> Result<(), SendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        self.inner
            .send(payload)
            .map_err(|SendError((_, _, msg))| SendError(msg))?;
        self.side.sent(stamp, self.inner.len());
        Ok(())
    }

    pub async fn send_async(&self, msg: T) -> Result<(), SendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        self.inner
            .send_async(payload)
            .await
            .map_err(|SendError((_, _, msg))| SendError(msg))?;
        self.side.sent(stamp, self.inner.len());
        Ok(())
    }

    pub fn try_send(&self, msg: T) -> Result<(), TrySendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        self.inner.try_send(payload).map_err(|e| match e {
            TrySendError::Full((_, _, msg)) => TrySendError::Full(msg),
            TrySendError::Disconnected((_, _, msg)) => TrySendError::Disconnected(msg),
        })?;
        self.side.sent(stamp, self.inner.len());
        Ok(())
    }

    pub fn send_timeout(
        &self,
        msg: T,
        timeout: std::time::Duration,
    ) -> Result<(), SendTimeoutError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        self.inner
            .send_timeout(payload, timeout)
            .map_err(|e| match e {
                SendTimeoutError::Timeout((_, _, msg)) => SendTimeoutError::Timeout(msg),
                SendTimeoutError::Disconnected((_, _, msg)) => SendTimeoutError::Disconnected(msg),
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

/// Instrumented flume [`flume::Receiver`] wrapper.
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

    pub fn recv(&self) -> Result<T, RecvError> {
        Ok(self.received(self.inner.recv()?))
    }

    pub async fn recv_async(&self) -> Result<T, RecvError> {
        Ok(self.received(self.inner.recv_async().await?))
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        Ok(self.received(self.inner.try_recv()?))
    }

    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Result<T, RecvTimeoutError> {
        Ok(self.received(self.inner.recv_timeout(timeout)?))
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

impl_receiver_iters!("flume");

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
            flume::bounded::<Payload<T>>(capacity),
        ),
        None => (ChannelType::Unbounded, flume::unbounded::<Payload<T>>()),
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
