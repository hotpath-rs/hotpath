//! Endpoint wrappers behind the `channel!` macro, one submodule per backend, plus
//! the instrumentation plumbing they share. Each backend keeps its own method
//! surface; stamping, event emission, clone counting and depth tracking live here.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::channels::{
    anchor_first_msg, entry_msg_counter, mark_closed, register_channel, sample_stamp,
    send_channel_event, ChannelEvent, ChannelType, Instant,
};

/// What a rebuilt inner channel carries: `(msg_id, send_ts, T)`. Monotonic `msg_id`
/// pairs a send with its matching receive; `send_ts` is `Some` for sampled messages.
pub(crate) type Payload<T> = (u64, Option<Instant>, T);

pub(crate) type LogFn<T> = fn(&T) -> String;

/// Event half of a stamped message, held by the sender until the push succeeds.
pub(crate) struct Stamp {
    msg_id: u64,
    sent_at: Option<Instant>,
    log: Option<String>,
}

/// Message-id source of one channel instance. Aggregated instances share one
/// sequence so ids stay unique within the entry; iter-mode instances keep a
/// local counter.
pub(crate) fn msg_counter(id: u32, iter: bool) -> Arc<AtomicU64> {
    if iter {
        Arc::new(AtomicU64::new(0))
    } else {
        entry_msg_counter(id)
    }
}

/// Wraps `msg` into its payload. Must run before the push: a consumer could
/// receive and timestamp the message the instant it is enqueued, so stamping
/// after would race the receive and read recv < send. For blocking or awaited
/// sends the stamp precedes the wait, so the delay includes backpressure.
#[inline]
pub(crate) fn stamp<T>(
    next_id: &AtomicU64,
    log_fn: Option<LogFn<T>>,
    msg: T,
) -> (Payload<T>, Stamp) {
    let log = log_fn.map(|f| f(&msg));
    let msg_id = next_id.fetch_add(1, Ordering::Relaxed);
    let sent_at = sample_stamp();
    (
        (msg_id, sent_at, msg),
        Stamp {
            msg_id,
            sent_at,
            log,
        },
    )
}

#[inline]
pub(crate) fn emit_sent(id: u32, stamp: Stamp, queue_len: usize) {
    send_channel_event(ChannelEvent::WrapMessageSent {
        id,
        msg_id: stamp.msg_id,
        log: stamp.log,
        timestamp: anchor_first_msg(stamp.msg_id, stamp.sent_at),
        queue_len,
    });
}

/// A `Some` payload stamp means the message is sampled: stamp `now` and report the
/// exact send->receive delay. `send_ts` is taken before the push, so it is `<= now`.
#[inline]
pub(crate) fn emit_received(id: u32, msg_id: u64, send_ts: Option<Instant>, queue_len: usize) {
    let (timestamp, delay_nanos) = match send_ts {
        Some(ts) => {
            let now = Instant::now();
            (Some(now), Some(now.duration_since(ts).as_nanos() as u64))
        }
        None => (None, None),
    };
    send_channel_event(ChannelEvent::WrapMessageReceived {
        id,
        msg_id,
        timestamp,
        queue_len,
        delay_nanos,
    });
}

/// State shared by every sender handle of one multi-message channel instance.
///
/// Deliberately not `Clone`: [`Self::clone_handle`] takes a `sender_count` slot,
/// which [`Self::drop_handle`] must give back exactly once.
pub(crate) struct SendSide<T> {
    id: u32,
    sender_count: Arc<AtomicUsize>,
    next_id: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
    log_fn: Option<LogFn<T>>,
}

impl<T> SendSide<T> {
    pub(crate) fn register(
        source: &'static str,
        label: Option<String>,
        channel_type: ChannelType,
        log_fn: Option<LogFn<T>>,
        iter: bool,
    ) -> Self {
        let id = register_channel::<T>(source, label, channel_type, iter);
        Self {
            id,
            sender_count: Arc::new(AtomicUsize::new(1)),
            next_id: msg_counter(id, iter),
            closed: Arc::new(AtomicBool::new(false)),
            log_fn,
        }
    }

    pub(crate) fn recv_side(&self) -> RecvSide {
        RecvSide {
            id: self.id,
            closed: Arc::clone(&self.closed),
        }
    }

    /// The caller pushes the payload, then reports success via [`Self::sent`].
    #[inline]
    pub(crate) fn prepare(&self, msg: T) -> (Payload<T>, Stamp) {
        stamp(&self.next_id, self.log_fn, msg)
    }

    #[inline]
    pub(crate) fn sent(&self, stamp: Stamp, queue_len: usize) {
        emit_sent(self.id, stamp, queue_len);
    }

    pub(crate) fn clone_handle(&self) -> Self {
        self.sender_count.fetch_add(1, Ordering::Relaxed);
        self.share()
    }

    /// `remaining` (messages still queued) is only read when this was the last handle.
    pub(crate) fn drop_handle(&self, remaining: impl FnOnce() -> usize) {
        if self.sender_count.fetch_sub(1, Ordering::AcqRel) == 1 {
            mark_closed(&self.closed, self.id, remaining());
        }
    }

    fn share(&self) -> Self {
        Self {
            id: self.id,
            sender_count: Arc::clone(&self.sender_count),
            next_id: Arc::clone(&self.next_id),
            closed: Arc::clone(&self.closed),
            log_fn: self.log_fn,
        }
    }
}

/// [`SendSide`] of a weak sender handle: holds no `sender_count` slot, so it has no
/// drop counterpart and dropping it never emits `Closed`.
#[cfg(feature = "tokio")]
pub(crate) struct WeakSendSide<T>(SendSide<T>);

#[cfg(feature = "tokio")]
impl<T> SendSide<T> {
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    pub(crate) fn downgrade(&self) -> WeakSendSide<T> {
        WeakSendSide(self.share())
    }
}

#[cfg(feature = "tokio")]
impl<T> WeakSendSide<T> {
    pub(crate) fn id(&self) -> u32 {
        self.0.id
    }

    /// Takes a `sender_count` slot unless the count already reached zero. Zero means
    /// the last strong sender's drop has emitted `Closed` - the state is terminal, so
    /// the upgrade must fail rather than revive the channel. CAS instead of
    /// `fetch_add` so the check and the increment are one atomic step.
    pub(crate) fn upgrade(&self) -> Option<SendSide<T>> {
        let sender_count = &self.0.sender_count;
        let mut count = sender_count.load(Ordering::Acquire);
        loop {
            if count == 0 {
                return None;
            }
            match sender_count.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(self.0.share()),
                Err(current) => count = current,
            }
        }
    }
}

#[cfg(feature = "tokio")]
impl<T> Clone for WeakSendSide<T> {
    fn clone(&self) -> Self {
        Self(self.0.share())
    }
}

/// Receiver-side identity of one channel instance; `closed` is the flag shared
/// with the instance's senders.
#[derive(Clone)]
pub(crate) struct RecvSide {
    id: u32,
    closed: Arc<AtomicBool>,
}

impl RecvSide {
    #[cfg(any(feature = "tokio", feature = "futures"))]
    pub(crate) fn new(id: u32, closed: Arc<AtomicBool>) -> Self {
        Self { id, closed }
    }

    #[cfg(any(feature = "tokio", feature = "futures"))]
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    #[inline]
    pub(crate) fn received(&self, msg_id: u64, send_ts: Option<Instant>, queue_len: usize) {
        emit_received(self.id, msg_id, send_ts, queue_len);
    }

    pub(crate) fn mark_closed(&self, remaining: usize) {
        mark_closed(&self.closed, self.id, remaining);
    }

    /// Teardown of a oneshot whose delivered value nobody will take. The sender
    /// emitted no `Closed` after delivering, so this must, then retire the value.
    #[cfg(any(feature = "tokio", feature = "futures"))]
    pub(crate) fn mark_closed_undelivered(&self) {
        self.closed.store(true, Ordering::Release);
        send_channel_event(ChannelEvent::Closed { id: self.id });
        send_channel_event(ChannelEvent::Abandoned {
            id: self.id,
            count: 1,
        });
    }
}

/// Self-tracked queue depth for backends without a cheap exact `len()`, shared by
/// both ends of an instance: incremented before each push (rolled back if the push
/// fails) and decremented after each receive. Counting before the push keeps the
/// counter non-negative - the channel's send->recv edge orders a producer's `+1`
/// ahead of the consumer's matching `-1`.
///
/// With `capacity` set, reported depths are clamped to it: a producer unblocked by
/// a consumer may apply its `+1` before that consumer applies its `-1`, so the raw
/// counter can transiently read `capacity + 1`.
#[derive(Clone)]
pub(crate) struct Depth {
    len: Arc<AtomicUsize>,
    capacity: Option<usize>,
}

impl Depth {
    pub(crate) fn new(capacity: Option<usize>) -> Self {
        Self {
            len: Arc::new(AtomicUsize::new(0)),
            capacity,
        }
    }

    /// Counts a message about to be pushed.
    #[inline]
    #[must_use = "dropping the slot rolls the reservation back"]
    pub(crate) fn reserve(&self) -> DepthSlot<'_> {
        DepthSlot {
            len: &self.len,
            queue_len: self.clamp(self.len.fetch_add(1, Ordering::Relaxed) + 1),
        }
    }

    /// Uncounts a received message and returns the depth left behind.
    #[inline]
    pub(crate) fn release(&self) -> usize {
        self.clamp(self.len.fetch_sub(1, Ordering::Relaxed) - 1)
    }

    pub(crate) fn load(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    #[inline]
    fn clamp(&self, queue_len: usize) -> usize {
        match self.capacity {
            Some(capacity) => queue_len.min(capacity),
            None => queue_len,
        }
    }
}

/// A [`Depth`] increment awaiting its push. Rolls back on drop unless
/// [`Self::commit`]ted, so a failed push, or a send future cancelled while parked
/// on a full channel, leaves no residue - a leaked `+1` would both skew `queue_len`
/// and inflate the `Abandoned` count emitted at instance teardown.
pub(crate) struct DepthSlot<'a> {
    len: &'a AtomicUsize,
    queue_len: usize,
}

impl DepthSlot<'_> {
    /// The push succeeded; returns the depth including this message.
    #[inline]
    pub(crate) fn commit(self) -> usize {
        let queue_len = self.queue_len;
        std::mem::forget(self);
        queue_len
    }
}

impl Drop for DepthSlot<'_> {
    fn drop(&mut self) {
        self.len.fetch_sub(1, Ordering::Relaxed);
    }
}

/// One impl per supported `(Sender, Receiver)` tuple; `channel!` reaches it through
/// the blanket [`crate::channels::InstrumentChannelWrap`] /
/// [`crate::channels::InstrumentChannelWrapLog`] impls, which only differ in `log_fn`.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "channel type `{Self}` cannot be instrumented by `channel!`",
    note = "supported: `(Sender, Receiver)` tuples from std, crossbeam, flume, async-channel, tokio (mpsc, oneshot) and futures_channel (mpsc, oneshot)"
)]
pub trait WrapChannel {
    type Msg;
    type Output;
    fn wrap(
        self,
        source: &'static str,
        label: Option<String>,
        capacity: Option<usize>,
        log_fn: Option<fn(&Self::Msg) -> String>,
        iter: bool,
    ) -> Self::Output;
}

/// Generates the receiver iterator trio shared by the blocking backends. Expects a
/// `Receiver<T>` in scope with `recv`/`try_recv` taking `&self`.
macro_rules! impl_receiver_iters {
    ($backend:literal) => {
        #[doc = concat!("Blocking iterator over a [`Receiver`], mirroring `", $backend, "::Iter`.")]
        pub struct Iter<'a, T> {
            rx: &'a Receiver<T>,
        }

        impl<T> Iterator for Iter<'_, T> {
            type Item = T;
            fn next(&mut self) -> Option<T> {
                self.rx.recv().ok()
            }
        }

        #[doc = concat!("Non-blocking iterator over a [`Receiver`], mirroring `", $backend, "::TryIter`.")]
        pub struct TryIter<'a, T> {
            rx: &'a Receiver<T>,
        }

        impl<T> Iterator for TryIter<'_, T> {
            type Item = T;
            fn next(&mut self) -> Option<T> {
                self.rx.try_recv().ok()
            }
        }

        #[doc = concat!("Owning blocking iterator, mirroring `", $backend, "::IntoIter`.")]
        pub struct IntoIter<T> {
            rx: Receiver<T>,
        }

        impl<T> Iterator for IntoIter<T> {
            type Item = T;
            fn next(&mut self) -> Option<T> {
                self.rx.recv().ok()
            }
        }

        impl<T> Receiver<T> {
            pub fn iter(&self) -> Iter<'_, T> {
                Iter { rx: self }
            }

            pub fn try_iter(&self) -> TryIter<'_, T> {
                TryIter { rx: self }
            }
        }

        impl<T> IntoIterator for Receiver<T> {
            type Item = T;
            type IntoIter = IntoIter<T>;
            fn into_iter(self) -> IntoIter<T> {
                IntoIter { rx: self }
            }
        }

        impl<'a, T> IntoIterator for &'a Receiver<T> {
            type Item = T;
            type IntoIter = Iter<'a, T>;
            fn into_iter(self) -> Iter<'a, T> {
                self.iter()
            }
        }
    };
}

#[cfg(feature = "async-channel")]
pub(crate) mod asc_wrap;
#[cfg(feature = "crossbeam")]
pub(crate) mod crossbeam_wrap;
#[cfg(feature = "flume")]
pub(crate) mod flume_wrap;
#[cfg(feature = "futures")]
pub(crate) mod ftc_oneshot_wrap;
#[cfg(feature = "futures")]
pub(crate) mod ftc_wrap;
pub(crate) mod std_wrap;
#[cfg(feature = "tokio")]
pub(crate) mod tokio_oneshot_wrap;
#[cfg(feature = "tokio")]
pub(crate) mod tokio_wrap;
