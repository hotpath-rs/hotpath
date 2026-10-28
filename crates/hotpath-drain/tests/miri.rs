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
