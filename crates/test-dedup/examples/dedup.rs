//! Removes duplicates from a list of numbers generated from a fixed seed.
//!
//! Run with:
//!   cargo run --release -p test-dedup --example dedup --features hotpath-alloc

use std::collections::HashSet;
use std::hint::black_box;

const LEN: usize = 50_000;
const DISTINCT: u64 = 10_000;
const ROUNDS: usize = 10;

fn input() -> Vec<u64> {
    let mut state: u64 = 42;
    (0..LEN)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) % DISTINCT
        })
        .collect()
}

#[hotpath::measure]
fn dedup(values: &[u64]) -> Vec<u64> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for &value in values {
        if seen.insert(value) {
            unique.push(value);
        }
    }
    unique
}

#[hotpath::main]
fn main() {
    let values = input();
    for _ in 0..ROUNDS {
        black_box(dedup(black_box(&values)));
    }
}
