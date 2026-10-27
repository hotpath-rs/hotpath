//! `published_prefix` across a cap-truncated sweep and a queue released after
//! the snapshot.
//!
//! Run with:
//!   cargo test -p hotpath-drain --features hotpath-ordered-drain --test ordered_drain

#![cfg(feature = "hotpath-ordered-drain")]

use std::sync::mpsc;
use std::thread;

use hotpath_drain::{EventProducer, EventQueueRegistry, CHUNK_SIZE};

static REGISTRY: EventQueueRegistry<u64> = EventQueueRegistry::new();

thread_local! {
    static PRODUCER: EventProducer<u64> = REGISTRY.register();
}

#[test]
fn prefix_waits_for_capped_sweep_and_survives_queue_release() {
    // One sweep walks at most 1024 chunks per queue, so this needs two.
    const EVENTS: u64 = CHUNK_SIZE as u64 * 1100;

    REGISTRY.set_active(true);
    let (published_tx, published_rx) = mpsc::channel();
    let (exit_tx, exit_rx) = mpsc::channel::<()>();
    let producer = thread::spawn(move || {
        for seq in 0..EVENTS {
            PRODUCER.with(|p| p.push(seq));
        }
        published_tx.send(()).unwrap();
        exit_rx.recv().unwrap();
    });

    published_rx.recv().unwrap();
    let prefix = REGISTRY.published_prefix();
    // Outside the snapshot: registered and published after it was taken.
    PRODUCER.with(|p| p.push(EVENTS));
    exit_tx.send(()).unwrap();
    producer.join().unwrap();

    let mut out = Vec::new();
    REGISTRY.sweep(&mut out);
    assert!((out.len() as u64) < EVENTS);
    assert!(!prefix.is_processed());

    // Reaches the closed producer's tail, so the registry releases its queue;
    // the prefix keeps it alive and still sees it as fully drained.
    REGISTRY.sweep(&mut out);
    assert_eq!(out.len() as u64, EVENTS + 1);
    assert!(prefix.is_processed());
}
