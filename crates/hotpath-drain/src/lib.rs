//! Lock-free event transport between instrumented threads and background workers.
//!
//! Each producing thread owns a ring of fixed-size chunks: it appends events
//! into the tail chunk with plain stores and publishes them with a single
//! `Release` store of the chunk's `len` - no mutex, no RMW atomic on the hot
//! path. When the tail chunk fills up, the producer moves to the next chunk in
//! the ring if the consumer has already drained it, and splices in a freshly
//! allocated chunk only when every chunk still holds unconsumed events. The
//! ring therefore grows to the peak number of chunks in flight between two
//! sweeps and then stops allocating: steady state is allocation-free.
//!
//! A single consumer per registry (the subsystem's background worker) sweeps
//! all registered queues periodically: it `Acquire`-loads each chunk's `len`,
//! reads the published prefix, resets fully consumed chunks and advances its
//! `head` past them. Producer and consumer touch disjoint chunks by
//! construction, so a queue is safe to drain at any moment - including queues
//! of threads parked at shutdown, which is what guarantees a complete final
//! report.
//!
//! Safety invariants:
//! - The ring is split in two arcs: `head..=tail` (live, holds published or
//!   in-progress events) and the rest (free, fully consumed). Only the owning
//!   thread writes slots and the `len`/`next` of its tail chunk, and only it
//!   rewrites `next` links inside the free arc when growing the ring.
//! - The consumer only reads slots below the published `len`, only writes
//!   `len` (resetting it to 0) after moving every event out of a full chunk,
//!   and only advances `head` after that reset.
//! - The producer reuses a chunk only after observing (`Acquire`) that `head`
//!   has moved past it, which orders the consumer's last reads and its reset
//!   before the producer's next writes. A stale `head` can only make the
//!   producer allocate unnecessarily, never reuse a live chunk.
//! - Single consumer per registry, enforced by the registry's internal mutex
//!   (locked only at thread registration and during sweeps - never on the
//!   event hot path).

use std::cell::{Cell, UnsafeCell};
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const CHUNK_SIZE: usize = 64;

/// Caps how many chunks one sweep drains from a single queue, so a producer
/// outpacing the consumer cannot pin the sweep in an endless tail chase.
/// Leftovers are picked up on the next tick.
const MAX_CHUNKS_PER_SWEEP: usize = 1024;

struct Chunk<M> {
    slots: [UnsafeCell<MaybeUninit<M>>; CHUNK_SIZE],
    /// Number of initialized slots; the producer publishes with `Release`,
    /// the consumer resets to 0 once it has moved every slot out.
    len: AtomicUsize,
    /// Ring link. Written by the producer when the chunk is allocated and
    /// when a new chunk is spliced in after it; never null.
    next: AtomicPtr<Chunk<M>>,
}

impl<M> Chunk<M> {
    fn new_raw(next: *mut Chunk<M>) -> *mut Chunk<M> {
        Box::into_raw(Box::new(Chunk {
            slots: [const { UnsafeCell::new(MaybeUninit::uninit()) }; CHUNK_SIZE],
            len: AtomicUsize::new(0),
            next: AtomicPtr::new(next),
        }))
    }
}

/// One thread's event queue: a ring of chunks. The owning thread appends via
/// its [`EventProducer`]; the registry's consumer drains.
pub struct EventQueue<M> {
    /// Oldest chunk with unconsumed events. Written by the consumer with
    /// `Release`; read by the producer once per chunk to decide whether the
    /// next chunk in the ring can be reused.
    head: AtomicPtr<Chunk<M>>,
    /// Consumed slot count within `head`. Consumer-only.
    consumed: AtomicUsize,
    /// Set by the producer's TLS drop so the consumer can drain the remainder
    /// and release the queue.
    closed: AtomicBool,
}

// SAFETY: the auto impls are lost to the raw chunk pointers. Sending or
// sharing the queue only moves `M` values across threads (hence `M: Send`);
// concurrent access is sound because producer and consumer touch disjoint
// chunks, synchronized by the `Release`/`Acquire` handoffs on `len` and
// `head` (see module-level safety invariants).
unsafe impl<M: Send> Send for EventQueue<M> {}
// SAFETY: same reasoning as `Send` above.
unsafe impl<M: Send> Sync for EventQueue<M> {}

impl<M> EventQueue<M> {
    /// Drains published events into `out`, walking at most `max_chunks` full
    /// chunks. Returns `true` when the drain reached the queue's tail, `false`
    /// when it stopped at the cap with chunks still pending. Must only be
    /// called by the single consumer (see registry).
    #[cfg_attr(
        feature = "hotpath-meta",
        hotpath_meta::measure(impl_type = "EventQueue")
    )]
    fn drain_into(&self, out: &mut Vec<M>, max_chunks: usize) -> bool {
        let mut chunk_ptr = self.head.load(Ordering::Relaxed);
        let mut consumed = self.consumed.load(Ordering::Relaxed);
        let mut chunks_walked = 0;
        let mut reached_tail = true;
        loop {
            // SAFETY: `chunk_ptr` is `head` or a `next` reached from it; every
            // chunk in the ring stays allocated until `EventQueue::drop`.
            let chunk = unsafe { &*chunk_ptr };
            let len = chunk.len.load(Ordering::Acquire);
            for i in consumed..len {
                // SAFETY: the `Acquire` load of `len` synchronizes with the
                // producer's `Release` publish, so slots `..len` are
                // initialized; slots below `consumed` were already read out
                // and are never touched twice (consumer-only cursor).
                out.push(unsafe { (*chunk.slots[i].get()).assume_init_read() });
            }
            consumed = len;
            if len == CHUNK_SIZE {
                if chunks_walked >= max_chunks {
                    reached_tail = false;
                    break;
                }
                // The producer links the next chunk before publishing this
                // one as full, so `next` is the chunk it moved on to.
                let next = chunk.next.load(Ordering::Acquire);
                // Every slot was moved out above; mark the chunk free for the
                // producer and leave it behind. The `Release` store of `head`
                // orders both the reads and this reset before any reuse.
                chunk.len.store(0, Ordering::Relaxed);
                chunk_ptr = next;
                consumed = 0;
                chunks_walked += 1;
                self.head.store(chunk_ptr, Ordering::Release);
                continue;
            }
            break;
        }
        self.consumed.store(consumed, Ordering::Relaxed);
        reached_tail
    }
}

impl<M> Drop for EventQueue<M> {
    fn drop(&mut self) {
        // Reached only after the producer is gone (closed) and the registry
        // released its Arc, so exclusive access is guaranteed. Live chunks
        // (from `head` to the tail) hold `consumed..len` events to drop; free
        // chunks have `len == 0`. Walk the ring once.
        let start = *self.head.get_mut();
        let mut chunk_ptr = start;
        let mut consumed = *self.consumed.get_mut();
        loop {
            // SAFETY: `&mut self` proves exclusive access; every chunk in the
            // ring is live and owned by this queue.
            let chunk = unsafe { &mut *chunk_ptr };
            let len = *chunk.len.get_mut();
            for i in consumed..len {
                // SAFETY: slots `consumed..len` were initialized by the
                // producer and never read out, so each holds a live `M` that
                // is dropped exactly once here.
                unsafe { (*chunk.slots[i].get()).assume_init_drop() };
            }
            consumed = 0;
            let next = *chunk.next.get_mut();
            // SAFETY: `chunk_ptr` came from `Box::into_raw` in
            // `Chunk::new_raw` and nothing can reference it after this drop
            // (exclusive access via `&mut self`).
            unsafe { drop(Box::from_raw(chunk_ptr)) };
            if next == start {
                break;
            }
            chunk_ptr = next;
        }
    }
}

/// The owning thread's write handle, stored in a `thread_local`.
pub struct EventProducer<M> {
    queue: Arc<EventQueue<M>>,
    tail: Cell<*mut Chunk<M>>,
    len: Cell<usize>,
}

impl<M> EventProducer<M> {
    /// Appends one event: a plain slot store plus a `Release` publish of the
    /// new length. Every `CHUNK_SIZE` events it moves to the next chunk of
    /// the ring, allocating only when the ring is full.
    #[inline]
    #[cfg_attr(
        feature = "hotpath-meta",
        hotpath_meta::measure(impl_type = "EventProducer")
    )]
    pub fn push(&self, m: M) {
        let tail = self.tail.get();
        let i = self.len.get();
        // SAFETY: `tail` is the producer-owned live tail chunk (the consumer
        // never advances past a chunk whose `len` it has not observed as
        // `CHUNK_SIZE`). Slot `i` is above the published `len`, so the
        // consumer cannot be reading it; the `Release` store of `len` below
        // publishes the write.
        unsafe { (*tail).slots[i].get().write(MaybeUninit::new(m)) };
        if i + 1 == CHUNK_SIZE {
            let next = self.next_chunk(tail);
            // SAFETY: `tail` is still live (see above); publishing the full
            // length is the last write to it before the consumer may take it.
            unsafe { (*tail).len.store(CHUNK_SIZE, Ordering::Release) };
            self.tail.set(next);
            self.len.set(0);
        } else {
            // SAFETY: as above.
            unsafe { (*tail).len.store(i + 1, Ordering::Release) };
            self.len.set(i + 1);
        }
    }

    /// Picks the chunk that follows the full `tail`: the next ring slot if
    /// the consumer has left it, otherwise a fresh chunk spliced in after
    /// `tail`. Must run before `tail` is published as full so the consumer
    /// finds the link in place.
    #[cold]
    fn next_chunk(&self, tail: *mut Chunk<M>) -> *mut Chunk<M> {
        // SAFETY: `tail` is live and its `next` link was written by this
        // thread; the ring is never broken, so `candidate` is a live chunk.
        let candidate = unsafe { (*tail).next.load(Ordering::Relaxed) };
        let head = self.queue.head.load(Ordering::Acquire);
        if candidate != head {
            // Free: the consumer advanced `head` past it (that `Release` store
            // is what the `Acquire` above synchronized with) after resetting
            // its `len` to 0, so no reads of it are pending.
            return candidate;
        }
        // Full ring: every chunk from `head` around to `tail` holds events
        // the consumer has not taken yet. Grow by one chunk after `tail`.
        let new = Chunk::new_raw(candidate);
        // SAFETY: only the producer writes `tail.next`, and the consumer reads
        // it only after `tail.len` is published as full, which happens after
        // this store.
        unsafe { (*tail).next.store(new, Ordering::Release) };
        new
    }
}

impl<M> Drop for EventProducer<M> {
    fn drop(&mut self) {
        self.queue.closed.store(true, Ordering::Release);
    }
}

/// All live per-thread queues for one event type, plus the gate that tells
/// producers whether a worker is consuming.
pub struct EventQueueRegistry<M> {
    active: AtomicBool,
    queues: Mutex<Vec<Arc<EventQueue<M>>>>,
}

impl<M: Send> Default for EventQueueRegistry<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: Send> EventQueueRegistry<M> {
    pub const fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            queues: Mutex::new(Vec::new()),
        }
    }

    /// Whether a worker is consuming. Producers check this before pushing so
    /// events cannot pile up unbounded when no one will ever drain them.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Release);
    }

    /// Creates and registers a queue for the calling thread. The ring starts
    /// as a single self-linked chunk and grows on demand.
    #[cfg_attr(
        feature = "hotpath-meta",
        hotpath_meta::measure(impl_type = "EventQueueRegistry")
    )]
    pub fn register(&self) -> EventProducer<M> {
        let first = Chunk::new_raw(ptr::null_mut());
        // SAFETY: `first` was just allocated and is not shared yet.
        unsafe { (*first).next.store(first, Ordering::Relaxed) };
        let queue = Arc::new(EventQueue {
            head: AtomicPtr::new(first),
            consumed: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
        });
        if let Ok(mut queues) = self.queues.lock() {
            queues.push(Arc::clone(&queue));
        }
        EventProducer {
            queue,
            tail: Cell::new(first),
            len: Cell::new(0),
        }
    }

    /// Drains every registered queue into `out` and releases queues whose
    /// producer thread has exited. Holding the registry lock for the whole
    /// sweep is what makes this the single consumer. Each queue is capped at
    /// `MAX_CHUNKS_PER_SWEEP`; leftovers are picked up on the next tick.
    #[cfg_attr(
        feature = "hotpath-meta",
        hotpath_meta::measure(impl_type = "EventQueueRegistry")
    )]
    pub fn sweep(&self, out: &mut Vec<M>) {
        self.sweep_inner(out, MAX_CHUNKS_PER_SWEEP);
    }

    /// Uncapped variant for the worker's final sweep at shutdown: there is no
    /// next tick to pick up leftovers, so every queue is drained to its tail.
    /// Terminates because producers are deactivated (`set_active(false)`)
    /// before shutdown is signalled, so queues can no longer grow.
    #[cfg_attr(
        feature = "hotpath-meta",
        hotpath_meta::measure(impl_type = "EventQueueRegistry")
    )]
    pub fn drain_all(&self, out: &mut Vec<M>) {
        self.sweep_inner(out, usize::MAX);
    }

    fn sweep_inner(&self, out: &mut Vec<M>, max_chunks: usize) {
        if let Ok(mut queues) = self.queues.lock() {
            queues.retain(|queue| {
                // Read `closed` before draining: the flag is set after the
                // producer's last push, so observing it guarantees the drain
                // below sees every event.
                let closed = queue.closed.load(Ordering::Acquire);
                let reached_tail = queue.drain_into(out, max_chunks);
                // A closed queue is released only once fully drained; a
                // cap-truncated drain keeps it for the next sweep so its tail
                // events are not freed unprocessed by `EventQueue::drop`.
                !(closed && reached_tail)
            });
        }
    }
}
