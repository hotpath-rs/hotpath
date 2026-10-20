//! Endpoint-wrapping `std::sync::mpsc` channel instrumentation for the `channel!` macro.
//!
//! Wraps the `Sender`/`SyncSender`/`Receiver` endpoints directly, so send/recv hit the
//! real channel.
//!
//! `std::sync::mpsc` exposes no public `len()`, so `queue_len` is read from a
//! self-maintained `AtomicUsize`: incremented before each publish (rolled back if the
//! send fails) and decremented after each receive. Counting before the publish is what
//! keeps the counter non-negative - the channel's send->recv edge orders a producer's
//! `+1` ahead of the consumer's matching `-1`, so a fast consumer can never decrement a
//! slot that has not yet been counted. The snapshot stays exact even under concurrent
//! producers.
//!
//! The inner channel carries `(msg_id, send_ts, T)`. Monotonic `msg_id` pairs a send
//! with its matching receive under multiple producers. `send_ts` is stamped before
//! publishing, so `send_ts <= recv_ts` always holds and the reported delay is
//! non-negative; for bounded (`SyncSender`) channels it precedes the blocking send, so
//! the delay includes backpressure wait. Both fields are internal - the public API
//! still uses `T`.
//!
//! The wrapper rebuilds the inner channel, so the `channel!` expression must be
//! constructed inline; endpoints cloned before wrapping are orphaned.
//!
//! For bounded channels (`SyncSender`) the rebuilt channel uses the `capacity = N` macro
//! argument, not the discarded `sync_channel(M)` you passed (std exposes no capacity
//! accessor to read `M`). The two must be equal: a mismatch changes the channel's
//! backpressure only in profiled builds (the `hotpath`-off `channel!` keeps your original
//! `M`), which can manifest as a deadlock that disappears when profiling is disabled.
//!
//! `std::sync::mpsc::Receiver` is single-consumer (not `Clone`), so there is exactly one
//! receiver and it emits `Closed` unconditionally on drop.
//!
//! Returns [`Sender`]/[`SyncSender`]/[`Receiver`], re-exported as
//! `hotpath::wrap::std::sync::mpsc::{Sender, SyncSender, Receiver}`.

use std::sync::mpsc::{
    self, Receiver as InnerReceiver, RecvError, RecvTimeoutError, SendError, Sender as InnerSender,
    SyncSender as InnerSyncSender, TryRecvError, TrySendError,
};

use crate::channels::wrapper::{Depth, LogFn, Payload, RecvSide, SendSide, WrapChannel};
use crate::channels::ChannelType;

/// Instrumented [`std::sync::mpsc::Sender`] (unbounded) wrapper.
///
/// Tracks every successful send and emits the self-tracked queue depth afterwards.
/// When the last clone is dropped, a `Closed` event is emitted.
pub struct Sender<T> {
    inner: InnerSender<Payload<T>>,
    side: SendSide<T>,
    depth: Depth,
}

impl<T> Sender<T> {
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

/// Instrumented [`std::sync::mpsc::SyncSender`] (bounded) wrapper.
///
/// Tracks every successful send and emits the self-tracked queue depth afterwards.
/// When the last clone is dropped, a `Closed` event is emitted.
pub struct SyncSender<T> {
    inner: InnerSyncSender<Payload<T>>,
    side: SendSide<T>,
    depth: Depth,
}

impl<T> SyncSender<T> {
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

    pub fn try_send(&self, msg: T) -> Result<(), TrySendError<T>> {
        let (payload, stamp) = self.side.prepare(msg);
        let slot = self.depth.reserve();
        match self.inner.try_send(payload) {
            Ok(()) => {
                self.side.sent(stamp, slot.commit());
                Ok(())
            }
            Err(TrySendError::Full((_, _, msg))) => Err(TrySendError::Full(msg)),
            Err(TrySendError::Disconnected((_, _, msg))) => Err(TrySendError::Disconnected(msg)),
        }
    }
}

impl<T> Clone for SyncSender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            side: self.side.clone_handle(),
            depth: self.depth.clone(),
        }
    }
}

impl<T> Drop for SyncSender<T> {
    fn drop(&mut self) {
        self.side.drop_handle(|| self.depth.load());
    }
}

/// Instrumented [`std::sync::mpsc::Receiver`] (single consumer) wrapper.
///
/// Tracks every successful receive and emits the self-tracked queue depth afterwards.
/// `std` receivers are not `Clone`, so this is the sole consumer; it emits `Closed`
/// unconditionally on drop.
pub struct Receiver<T> {
    inner: InnerReceiver<Payload<T>>,
    side: RecvSide,
    depth: Depth,
}

impl<T> Receiver<T> {
    fn received(&self, (msg_id, send_ts, msg): Payload<T>) -> T {
        self.side.received(msg_id, send_ts, self.depth.release());
        msg
    }

    pub fn recv(&self) -> Result<T, RecvError> {
        Ok(self.received(self.inner.recv()?))
    }

    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        Ok(self.received(self.inner.try_recv()?))
    }

    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Result<T, RecvTimeoutError> {
        Ok(self.received(self.inner.recv_timeout(timeout)?))
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.side.mark_closed(self.depth.load());
    }
}

impl_receiver_iters!("std::sync::mpsc");

fn build_unbounded<T>(
    source: &'static str,
    label: Option<String>,
    log_fn: Option<LogFn<T>>,
    iter: bool,
) -> (Sender<T>, Receiver<T>) {
    let side = SendSide::register(source, label, ChannelType::Unbounded, log_fn, iter);
    let depth = Depth::new(None);
    let (tx, rx) = mpsc::channel::<Payload<T>>();
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

fn build_bounded<T>(
    source: &'static str,
    label: Option<String>,
    capacity: Option<usize>,
    log_fn: Option<LogFn<T>>,
    iter: bool,
) -> (SyncSender<T>, Receiver<T>) {
    let Some(capacity) = capacity else {
        panic!("bounded std::sync::mpsc wrap requires `capacity = N` (std exposes no capacity accessor); it must match the sync_channel(N) argument, e.g. channel!(mpsc::sync_channel::<T>(100), capacity = 100)");
    };
    let side = SendSide::register(source, label, ChannelType::Bounded(capacity), log_fn, iter);
    let depth = Depth::new(Some(capacity));
    let (tx, rx) = mpsc::sync_channel::<Payload<T>>(capacity);
    let receiver = Receiver {
        inner: rx,
        side: side.recv_side(),
        depth: depth.clone(),
    };
    let sender = SyncSender {
        inner: tx,
        side,
        depth,
    };
    (sender, receiver)
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
        build_unbounded(source, label, log_fn, iter)
    }
}

impl<T: Send + 'static> WrapChannel for (InnerSyncSender<T>, InnerReceiver<T>) {
    type Msg = T;
    type Output = (SyncSender<T>, Receiver<T>);
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
