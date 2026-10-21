//! Different bottleneck types
//!
//! Profile with hotpath:
//! ```bash
//! cargo run -p test-tokio-async --example overview --features='hotpath-alloc,hotpath-cpu'
//! ```

use std::hint::black_box;
use std::time::Duration;

#[hotpath::measure]
fn sync_work() {
    let mut result: u64 = 1;
    for i in 0..200000 {
        result = result.wrapping_mul(black_box(i as u64).wrapping_add(7));
        result ^= result >> 3;
    }
}

#[hotpath::measure]
fn sync_alloc() {
    let buf: Vec<u8> = vec![1; 1024];
    std::hint::black_box(&buf);
}

#[hotpath::measure]
async fn async_sleep() {
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::main]
#[hotpath::main]
async fn main() {
    for _ in 0..1000 {
        sync_work();
        sync_alloc();
        async_sleep().await;
    }
}
