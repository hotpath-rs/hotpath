//! Run with:
//!   cargo run -p test-tokio-async --example cpu_park_vs_busy --features hotpath-cpu
//!
//! Single-thread example for comparing hotpath-cpu output with samply UI.
//!

use std::hint::black_box;
use std::thread;
use std::time::{Duration, Instant};

const COMPUTE_BATCH: u32 = 200_000;

#[hotpath::measure]
fn busy_compute(iterations: u32) -> u64 {
    let mut result: u64 = 1;
    for i in 0..iterations {
        result = result.wrapping_mul(black_box(i as u64).wrapping_add(7));
        result ^= result >> 3;
    }
    result
}

#[hotpath::measure]
fn park_main(duration: Duration) {
    thread::sleep(duration);
}

#[hotpath::main]
fn main() {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        black_box(busy_compute(COMPUTE_BATCH));
    }

    park_main(Duration::from_secs(5));
}
