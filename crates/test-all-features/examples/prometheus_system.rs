//! Threads + tokio runtime + gauge! workload for the Prometheus exporter
//! integration test: a 2-worker tokio runtime with runtime sampling on, a
//! named busy thread for the thread monitor to sample, and two gauges with
//! known values. Set TEST_SLEEP_SECONDS to keep the process (and the
//! exporter) alive after the workload completes.
//!
//! Run with:
//!   cargo run -p test-all-features --example prometheus_system --features hotpath-prometheus

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
#[hotpath::main]
async fn main() {
    hotpath::tokio_runtime!();

    hotpath::gauge!("test-gauge").set(42.0);
    hotpath::gauge!("queue-depth").set(10.0).inc(5.0).dec(3.0);

    // A named thread busy long enough for the 250ms thread monitor to sample
    // it with nonzero CPU.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let busy = std::thread::Builder::new()
        .name("busy-worker".into())
        .spawn(move || {
            let mut x = 0u64;
            while !stop_flag.load(Ordering::Relaxed) {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                std::hint::black_box(x);
            }
        })
        .expect("spawn busy thread");

    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(800)).await;

    if let Ok(secs) = std::env::var("TEST_SLEEP_SECONDS") {
        if let Ok(secs) = secs.parse::<u64>() {
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    }

    stop.store(true, Ordering::Relaxed);
    busy.join().expect("busy thread");
    println!("prometheus system example completed");
}
