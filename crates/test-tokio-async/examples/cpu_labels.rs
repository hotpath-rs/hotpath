//! Validates CPU attribution for `#[measure(label = "...")]` functions.
//!
//! Run:
//! ```bash
//! cargo run -p test-tokio-async --example cpu_labels \
//!   --features 'hotpath-cpu' --release
//! ```

use std::hint::black_box;

#[hotpath::measure(label = "custom_heavy")]
#[inline(never)]
fn heavy_with_label(iterations: u32) -> u64 {
    let mut result: u64 = 1;
    for i in 0..iterations {
        result = result.wrapping_mul(black_box(i as u64).wrapping_add(7));
        result ^= result >> 3;
    }
    result
}

#[hotpath::measure]
#[inline(never)]
fn heavy_no_label(iterations: u32) -> u64 {
    let mut result: u64 = 1;
    for i in 0..iterations {
        result = result.wrapping_mul(black_box(i as u64).wrapping_add(7));
        result ^= result >> 3;
    }
    result
}

#[hotpath::main]
fn main() {
    let mut total: u64 = 0;
    for _ in 0..2000 {
        total = total.wrapping_add(heavy_with_label(50_000));
        total = total.wrapping_add(heavy_no_label(50_000));
    }
    black_box(total);
}
