//! Lock-free event transport between instrumented threads and background workers.
//!
//! Each producing thread owns a chunked SPSC queue: it appends events into
//! fixed-size chunks with plain stores and publishes them with a single
//! `Release` store of the chunk's `len` - no mutex, no RMW atomic on the hot
//! path. Chunks form a linked list, so the queue is unbounded.
//!
//! A single consumer per registry (the subsystem's background worker) sweeps
//! all registered queues periodically: it `Acquire`-loads each chunk's `len`,
//! reads the published prefix, and frees fully consumed chunks. Producer and
//! consumer touch disjoint slot ranges by construction, so a queue is safe to
//! drain at any moment - including queues of threads parked at shutdown,
//! which is what guarantees a complete final report.
//!
//! Safety invariants:
//! - Only the owning thread writes slots and the `len`/`next` of its tail chunk.
//! - The consumer only reads slots below the published `len` and only frees a
//!   chunk after fully consuming it and observing its `next` pointer.
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
    /// Number of initialized slots; the producer publishes with `Release`.
    len: AtomicUsize,
    /// Set once by the producer when the chunk is full; never unset.
    next: AtomicPtr<Chunk<M>>,
}

impl<M> Chunk<M> {
    fn new_raw() -> *mut Chunk<M> {
        Box::into_raw(Box::new(Chunk {
            slots: [const { UnsafeCell::new(MaybeUninit::uninit()) }; CHUNK_SIZE],
            len: AtomicUsize::new(0),
            next: AtomicPtr::new(ptr::null_mut()),
        }))
    }
}

/// One thread's event queue: a linked list of chunks. The owning thread
/// appends via its [`EventProducer`]; the registry's consumer drains.
pub struct EventQueue<M> {
    /// Oldest chunk with unconsumed events. Consumer-owned after creation.
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
// slot ranges, synchronized by the `Release`/`Acquire` handoff on `len`
// (see module-level safety invariants).
unsafe impl<M: Send> Send for EventQueue<M> {}
// SAFETY: same reasoning as `Send` above.
unsafe impl<M: Send> Sync for EventQueue<M> {}

impl<M> EventQueue<M> {
    /// Drains published events into `out`, walking at most `max_chunks` full
    /// chunks. Returns `true` when the drain reached the queue's tail, `false`
    /// when it stopped at the cap with chunks still pending. Must only be
    /// called by the single consumer (see registry).
    fn drain_into(&self, out: &mut Vec<M>, max_chunks: usize) -> bool {
        let mut chunk_ptr = self.head.load(Ordering::Relaxed);
        let mut consumed = self.consumed.load(Ordering::Relaxed);
        let mut chunks_walked = 0;
        let mut reached_tail = true;
        loop {
            // SAFETY: `chunk_ptr` is either `head` (never null, freed only by
            // this single consumer after advancing past it) or a non-null
            // `next` observed below; the chunk stays alive until this loop
            // frees it.
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
                let next = chunk.next.load(Ordering::Acquire);
                if !next.is_null() {
                    if chunks_walked >= max_chunks {
                        reached_tail = false;
                        break;
                    }
                    // SAFETY: `chunk_ptr` came from `Box::into_raw` in
                    // `Chunk::new_raw`. The chunk is full (`len == CHUNK_SIZE`)
                    // and fully consumed, and the producer moved on to `next`,
                    // so neither side will touch it again; all `M` values were
                    // moved out above, so dropping the box frees only
                    // `MaybeUninit` storage.
                    unsafe { drop(Box::from_raw(chunk_ptr)) };
                    chunk_ptr = next;
                    consumed = 0;
                    chunks_walked += 1;
                    continue;
                }
            }
            break;
        }
        self.head.store(chunk_ptr, Ordering::Relaxed);
        self.consumed.store(consumed, Ordering::Relaxed);
        reached_tail
    }
}

impl<M> Drop for EventQueue<M> {
    fn drop(&mut self) {
        // Reached only after the producer is gone (closed) and the registry
        // released its Arc, so exclusive access is guaranteed.
        let mut chunk_ptr = *self.head.get_mut();
        let mut consumed = *self.consumed.get_mut();
        while !chunk_ptr.is_null() {
            // SAFETY: `&mut self` proves exclusive access; every chunk from
            // `head` onward is live and owned by this queue.
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
    /// new length. Allocates a fresh chunk every `CHUNK_SIZE` events.
    #[inline]
    pub fn push(&self, m: M) {
        let tail = self.tail.get();
        let i = self.len.get();
        // SAFETY: `tail` is the producer-owned live tail chunk (the consumer
        // never frees a chunk whose `next` it has not observed, and `next` is
        // set only after this chunk is full). Slot `i` is above the published
        // `len`, so the consumer cannot be reading it; the `Release` store of
        // `len` publishes the write.
        unsafe {
            (*tail).slots[i].get().write(MaybeUninit::new(m));
            (*tail).len.store(i + 1, Ordering::Release);
        }
        if i + 1 == CHUNK_SIZE {
            let new = Chunk::new_raw();
            // SAFETY: `tail` is still live (see above); only the producer
            // writes `next`, and only once, when the chunk is full.
            unsafe { (*tail).next.store(new, Ordering::Release) };
            self.tail.set(new);
            self.len.set(0);
        } else {
            self.len.set(i + 1);
        }
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

    /// Creates and registers a queue for the calling thread.
    pub fn register(&self) -> EventProducer<M> {
        let first = Chunk::new_raw();
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
    pub fn sweep(&self, out: &mut Vec<M>) {
        self.sweep_inner(out, MAX_CHUNKS_PER_SWEEP);
    }

    /// Uncapped variant for the worker's final sweep at shutdown: there is no
    /// next tick to pick up leftovers, so every queue is drained to its tail.
    /// Terminates because producers are deactivated (`set_active(false)`)
    /// before shutdown is signalled, so queues can no longer grow.
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
