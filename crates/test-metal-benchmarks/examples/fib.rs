//! Single-threaded CPU-bound benchmark: naive recursive Fibonacci. Only the
//! `fib_round` wrapper is measured, never the recursion itself. Input and
//! result go through `black_box` and the result is asserted, so the compiler
//! can neither fold nor drop the computation. No warm-up: every call is
//! measured.
//!
//! Run with:
//!   cargo run --release -p test-metal-benchmarks --example fib --features hotpath

use std::hint::black_box;

const N: u64 = 35;
const EXPECTED: u64 = 9_227_465;
const MEASURED_CALLS: usize = 20;

#[inline(never)]
fn fib(n: u64) -> u64 {
    if n < 2 {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

#[hotpath::measure]
fn fib_round() {
    let result = fib(black_box(N));
    assert_eq!(black_box(result), EXPECTED);
}

#[hotpath::main(percentiles = [50, 95])]
fn main() {
    for _ in 0..MEASURED_CALLS {
        fib_round();
    }
}
