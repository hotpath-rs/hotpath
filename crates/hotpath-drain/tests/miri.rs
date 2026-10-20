//! Minimal scenario for miri: several producer threads push boxed events
//! across chunk boundaries, the consumer sweeps repeatedly while the
//! producers are still publishing, then the remainder is drained after the
//! producers exit.
//!
//! Run with:
//!   cargo +nightly miri test -p hotpath-drain --test miri

use std::sync::{Arc, Barrier};
use std::thread;

use hotpath_drain::{EventProducer, EventQueueRegistry, CHUNK_SIZE};

static REGISTRY: EventQueueRegistry<Box<u64>> = EventQueueRegistry::new();

thread_local! {
    static PRODUCER: EventProducer<Box<u64>> = REGISTRY.register();
}

#[test]
fn push_sweep_drain_across_threads() {
    const PRODUCERS: u64 = 3;
    const PER_PRODUCER: u64 = CHUNK_SIZE as u64 * 3 + 5;
    const TOTAL: u64 = PRODUCERS * PER_PRODUCER;
    // Published before the first sweep: one full chunk plus a partial one.
    const BEFORE_SWEEP: u64 = CHUNK_SIZE as u64 + 3;

    REGISTRY.set_active(true);
    // Single rendezvous: producers park after `BEFORE_SWEEP` events so the
    // first sweep is guaranteed to find live, half-filled queues. After it
    // they keep publishing unsynchronized while the consumer keeps sweeping,
    // so the slot, `len` and `next` accesses genuinely overlap.
    let barrier = Arc::new(Barrier::new(PRODUCERS as usize + 1));
    let handles: Vec<_> = (0..PRODUCERS)
        .map(|producer| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let base = producer * PER_PRODUCER;
                for seq in 0..BEFORE_SWEEP {
                    PRODUCER.with(|p| p.push(Box::new(base + seq)));
                }
                barrier.wait();
                for seq in BEFORE_SWEEP..PER_PRODUCER {
                    PRODUCER.with(|p| p.push(Box::new(base + seq)));
                }
            })
        })
        .collect();

    let mut out = Vec::new();
    barrier.wait();
    REGISTRY.sweep(&mut out);
    assert!(out.len() as u64 >= PRODUCERS * BEFORE_SWEEP);
    // Producers are still running: sweep concurrently with their pushes
    // until every event has been observed. Terminates because the producers
    // publish a finite number of events.
    while (out.len() as u64) < TOTAL {
        REGISTRY.sweep(&mut out);
        thread::yield_now();
    }

    for handle in handles {
        handle.join().unwrap();
    }
    REGISTRY.set_active(false);
    // Nothing left to drain; releases the queues closed by the exited
    // producers.
    REGISTRY.drain_all(&mut out);

    let mut values: Vec<u64> = out.into_iter().map(|b| *b).collect();
    values.sort_unstable();
    let expected: Vec<u64> = (0..TOTAL).collect();
    assert_eq!(values, expected);
}

/// Wrap-around: the producer fills the ring several times over while the
/// consumer keeps up, so chunks are reused rather than allocated, and the
/// events still arrive complete and in per-producer order.
#[test]
fn ring_reuses_chunks_across_laps() {
    static REG: EventQueueRegistry<Box<u64>> = EventQueueRegistry::new();
    const LAPS: u64 = 4;
    const PER_LAP: u64 = CHUNK_SIZE as u64 * 3;

    REG.set_active(true);
    let producer = REG.register();
    let mut out = Vec::new();
    let mut all = Vec::new();
    for lap in 0..LAPS {
        for i in 0..PER_LAP {
            producer.push(Box::new(lap * PER_LAP + i));
        }
        // Consumer takes everything, resetting the chunks for the next lap.
        REG.sweep(&mut out);
        all.extend(out.drain(..).map(|b| *b));
    }
    drop(producer);
    REG.set_active(false);
    REG.drain_all(&mut out);
    all.extend(out.drain(..).map(|b| *b));
    let expected: Vec<u64> = (0..LAPS * PER_LAP).collect();
    assert_eq!(all, expected);
}

/// Growth: with no consumer sweeping, every chunk stays live and the ring
/// must grow; the final drain then returns everything in order.
#[test]
fn ring_grows_when_consumer_is_behind() {
    static REG: EventQueueRegistry<Box<u64>> = EventQueueRegistry::new();
    const TOTAL: u64 = CHUNK_SIZE as u64 * 5 + 7;

    REG.set_active(true);
    let producer = REG.register();
    for i in 0..TOTAL {
        producer.push(Box::new(i));
    }
    let mut out = Vec::new();
    REG.sweep(&mut out);
    let got: Vec<u64> = out.iter().map(|b| **b).collect();
    assert_eq!(got, (0..TOTAL).collect::<Vec<_>>());

    // Second burst reuses the grown ring, partially interleaved with sweeps.
    out.clear();
    for i in 0..TOTAL {
        producer.push(Box::new(i));
        if i % 100 == 0 {
            REG.sweep(&mut out);
        }
    }
    drop(producer);
    REG.set_active(false);
    REG.drain_all(&mut out);
    let got: Vec<u64> = out.iter().map(|b| **b).collect();
    assert_eq!(got, (0..TOTAL).collect::<Vec<_>>());
}

/// Drop: a queue with several live chunks of unconsumed events plus free
/// chunks in the ring is dropped; miri checks every `Box` is freed exactly
/// once and nothing leaks.
#[test]
fn unconsumed_events_dropped_with_ring() {
    let registry = EventQueueRegistry::<Box<u64>>::new();
    let producer = registry.register();
    let mut out = Vec::new();
    // Grow the ring to a few chunks, then free them all.
    for i in 0..(CHUNK_SIZE as u64 * 3) {
        producer.push(Box::new(i));
    }
    registry.sweep(&mut out);
    assert_eq!(out.len(), CHUNK_SIZE * 3);
    out.clear();
    // Leave two and a half chunks unconsumed, spanning reused chunks.
    for i in 0..(CHUNK_SIZE as u64 * 2 + CHUNK_SIZE as u64 / 2) {
        producer.push(Box::new(i));
    }
    drop(producer);
    drop(registry);
}
