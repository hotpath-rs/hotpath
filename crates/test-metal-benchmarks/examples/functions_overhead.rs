//! Instrumented function calls: `instrumented_op` is a `#[hotpath::measure]`
//! function doing fixed integer work (not a time-based spin, which would hide
//! CPU frequency changes). The measured `phase` calls it `OPS` times, `REPS`
//! times, after one unmeasured warm-up loop. The phase rows hold exactly `REPS`
//! calls; `instrumented_op` also counts its `OPS` warm-up calls.
//!
//! Run with:
//!   cargo run --release -p test-metal-benchmarks --example functions_overhead --features hotpath

use std::hint::black_box;

const OPS: u64 = 1_000_000;
const REPS: usize = 10;
const WORK_ROUNDS: u32 = 64;

/// Fixed work: `WORK_ROUNDS` dependent xorshift64 rounds. Unlike an affine
/// (multiply-add) step, these do not fold into one operation.
#[inline(never)]
fn work(x: u64) -> u64 {
    let mut v = black_box(x) | 1;
    for _ in 0..WORK_ROUNDS {
        v ^= v << 13;
        v ^= v >> 7;
        v ^= v << 17;
    }
    v
}

#[hotpath::measure]
fn instrumented_op(x: u64) -> u64 {
    work(x)
}

fn run_loop() -> u64 {
    let mut acc = 0;
    for i in 0..OPS {
        acc ^= instrumented_op(i);
    }
    black_box(acc)
}

#[hotpath::measure]
fn phase() -> u64 {
    run_loop()
}

#[hotpath::main(percentiles = [50, 95])]
fn main() {
    let expected = run_loop();
    for _ in 0..REPS {
        assert_eq!(phase(), expected);
    }
}
