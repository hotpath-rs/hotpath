//! A measured function that allocates and frees in its body, for comparing
//! its instruction count with and without allocation tracking.
//!
//! Run with:
//!   CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" cargo run --release -p test-callgrind --example allocs --features hotpath,hotpath-callgrind,hotpath-alloc

use std::hint::black_box;

#[hotpath::measure]
fn allocating() -> u64 {
    let mut acc = 0u64;
    for i in 0..1_000u64 {
        acc = acc.wrapping_add(*black_box(Box::new(i)));
    }
    acc
}

#[hotpath::main]
fn main() {
    black_box(allocating());
}
