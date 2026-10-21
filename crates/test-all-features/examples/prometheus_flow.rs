//! Deterministic channels + locks + streams + futures + alloc workload for
//! the Prometheus exporter integration tests: 50 messages through a wrapped
//! bounded channel, 20 mutex acquisitions, 15 rwlock reads + 5 writes, a
//! 7-item stream, 20 wrapped futures, 50 calls of a measured allocating
//! function. Set TEST_SLEEP_SECONDS to keep the process (and the exporter)
//! alive after the workload completes.
//!
//! Run with:
//!   cargo run -p test-all-features --example prometheus_flow --features hotpath-prometheus

use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;

#[hotpath::measure]
fn allocate_chunk(i: u64) -> Vec<u64> {
    vec![i; 128]
}

#[tokio::main(flavor = "current_thread")]
#[hotpath::main]
async fn main() {
    {
        let (tx, mut rx) =
            hotpath::channel!(tokio::sync::mpsc::channel::<u64>(10), label = "work-queue");

        let mutex = Arc::new(hotpath::mutex!(
            tokio::sync::Mutex::new(0u64),
            label = "counter"
        ));
        let rw_lock = Arc::new(hotpath::rw_lock!(
            tokio::sync::RwLock::new(0u64),
            label = "state"
        ));

        let producer = tokio::spawn(async move {
            for i in 0..50u64 {
                tx.send(i).await.expect("receiver dropped");
                tokio::time::sleep(Duration::from_micros(50)).await;
            }
        });

        for i in 0..50u64 {
            let msg = rx.recv().await.expect("sender dropped");
            if i % 5 == 0 {
                let mut v = mutex.lock().await;
                *v += msg;
                tokio::time::sleep(Duration::from_micros(20)).await;
            }
            if i % 10 == 0 {
                let mut w = rw_lock.write().await;
                *w += 1;
            } else if i % 3 == 0 {
                let r = rw_lock.read().await;
                std::hint::black_box(*r);
            }
        }
        producer.await.expect("producer");

        for _ in 0..10 {
            let mut v = mutex.lock().await;
            *v += 1;
        }

        let stream = hotpath::stream!(futures::stream::iter(1..=7), label = "number-stream");
        let numbers: Vec<i32> = stream.collect().await;
        std::hint::black_box(numbers);

        for i in 0..20u64 {
            let v = hotpath::future!(async move { i * 2 }, label = "doubler").await;
            std::hint::black_box(v);
        }

        for i in 0..50u64 {
            std::hint::black_box(allocate_chunk(i));
        }
    }

    if let Ok(secs) = std::env::var("TEST_SLEEP_SECONDS") {
        if let Ok(secs) = secs.parse::<u64>() {
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    }

    println!("prometheus flow example completed");
}
