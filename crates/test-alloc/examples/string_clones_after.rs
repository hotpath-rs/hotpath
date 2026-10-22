//! Fix for the per-message `String` clones shown in the `string_clones_before`
//! example: the payload is shared as `Arc<str>`, so sending it is a reference
//! count bump instead of a heap allocation. The alloc report drops to channel
//! bookkeeping only, and the threads report shows the `producer` thread's
//! allocations shrinking accordingly.
//!
//! Run with:
//!   cargo run --release -p test-alloc --example string_clones_after --features hotpath-alloc

use std::sync::{mpsc, Arc};

const MESSAGES: usize = 5_000_000;

// All messages share one heap buffer: a clone only bumps the refcount.
#[hotpath::measure]
fn send(payload: &Arc<str>, tx: &mpsc::SyncSender<Arc<str>>) {
    tx.send(Arc::clone(payload)).unwrap();
}

#[hotpath::measure]
fn process(message: Arc<str>) -> usize {
    std::hint::black_box(message.len())
}

#[hotpath::main(report = "functions-timing,functions-alloc,threads", threads_limit = 2)]
fn main() {
    let (tx, rx) = mpsc::sync_channel::<Arc<str>>(1024);

    let producer = std::thread::Builder::new()
        .name("producer".into())
        .spawn(move || {
            let payload: Arc<str> = Arc::from("x".repeat(1024));
            for _ in 0..MESSAGES {
                send(&payload, &tx);
            }
        })
        .unwrap();

    let consumer = std::thread::Builder::new()
        .name("consumer".into())
        .spawn(move || {
            let mut total = 0;
            while let Ok(message) = rx.recv() {
                total += process(message);
            }
            println!("processed {total} bytes");
        })
        .unwrap();

    producer.join().unwrap();
    consumer.join().unwrap();
}
