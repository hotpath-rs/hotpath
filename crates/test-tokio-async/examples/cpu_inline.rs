//! Compares CPU attribution for `#[inline(never)]` vs `#[inline(always)]`.
//!
//! Run:
//! ```bash
//! cargo run -p test-tokio-async --example cpu_inline \
//!   --features 'hotpath-cpu' --release
//! ```

use std::hint::black_box;

#[hotpath::measure]
#[inline(never)]
fn never_inlined(iterations: u32) -> u64 {
    let mut result: u64 = 1;
    for i in 0..iterations {
        result = result.wrapping_mul(black_box(i as u64).wrapping_add(7));
        result ^= result >> 3;
    }
    result
}

#[hotpath::measure]
#[inline(always)]
fn always_inlined(iterations: u32) -> u64 {
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
        total = total.wrapping_add(never_inlined(50_000));
        total = total.wrapping_add(always_inlined(50_000));
    }
    black_box(total);
}
