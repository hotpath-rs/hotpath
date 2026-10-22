//! Demonstrates the allocation cost of sending owned `String` copies through
//! a channel: every `send` call clones the 1 KB payload, so the alloc report
//! shows ~1 KB allocated per call and the threads report shows the `producer`
//! thread allocating ~5 GB. Compare with the `string_clones_after` example.
//!
//! Run with:
//!   cargo run --release -p test-alloc --example string_clones_before --features hotpath-alloc

use std::sync::mpsc;

const MESSAGES: usize = 5_000_000;

// Every message is an owned copy: one heap allocation + memcpy per send.
#[hotpath::measure]
fn send(payload: &str, tx: &mpsc::SyncSender<String>) {
    tx.send(payload.to_owned()).unwrap();
}

#[hotpath::measure]
fn process(message: String) -> usize {
    std::hint::black_box(message.len())
}

#[hotpath::main(report = "functions-timing,functions-alloc,threads", threads_limit = 2)]
fn main() {
    let (tx, rx) = mpsc::sync_channel::<String>(1024);

    let producer = std::thread::Builder::new()
        .name("producer".into())
        .spawn(move || {
            let payload: String = "x".repeat(1024);
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
