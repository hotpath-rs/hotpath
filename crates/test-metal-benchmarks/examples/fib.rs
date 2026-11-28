//! Single-threaded CPU-bound benchmark: naive recursive Fibonacci. Only the
//! `fib_round` wrapper is measured, never the recursion itself. Input and
//! result go through `black_box` and the result is asserted, so the compiler
//! can neither fold nor drop the computation.
//!
//! Run with:
//!   cargo run --release -p test-metal-benchmarks --example fib --features hotpath

use std::hint::black_box;

const N: u64 = 35;
const EXPECTED: u64 = 9_227_465;
const WARMUP_CALLS: usize = 3;
const MEASURED_CALLS: usize = 20;

#[inline(never)]
fn fib(n: u64) -> u64 {
    if n < 2 {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

#[inline(never)]
fn fib_round_raw() {
    let result = fib(black_box(N));
    assert_eq!(black_box(result), EXPECTED);
}

#[hotpath::measure]
fn fib_round() {
    fib_round_raw();
}

#[hotpath::main(percentiles = [50, 95])]
fn main() {
    for _ in 0..WARMUP_CALLS {
        fib_round_raw();
    }
    for _ in 0..MEASURED_CALLS {
        fib_round();
    }
}
