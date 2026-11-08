//! Follows the same pointer chain through two arrays of equal size: one links
//! each index to the next, the other to a seeded random one. Both run the same
//! instructions, but the random chain misses the CPU caches on nearly every
//! step, so it is far slower in wall-clock time.
//!
//! Run with:
//!   cargo run --release -p test-cache --example cache --features hotpath-alloc

use std::hint::black_box;

const LEN: usize = 1 << 22;
const ROUNDS: usize = 5;

/// `0 -> 1 -> 2 -> ... -> LEN - 1 -> 0`.
fn sequential_chain() -> Vec<u32> {
    (0..LEN as u32).map(|i| (i + 1) % LEN as u32).collect()
}

/// Every index visited once in a seeded random order, starting and ending at 0.
fn random_chain() -> Vec<u32> {
    let mut order: Vec<u32> = (0..LEN as u32).collect();
    let mut state: u64 = 42;
    // Fisher-Yates over `order[1..]`, so the cycle still starts at 0.
    for i in (2..LEN).rev() {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = 1 + ((state >> 33) as usize % i);
        order.swap(i, j);
    }
    let mut next = vec![0u32; LEN];
    for pair in order.windows(2) {
        next[pair[0] as usize] = pair[1];
    }
    next[order[LEN - 1] as usize] = 0;
    next
}

#[inline(never)]
fn traverse(next: &[u32], steps: usize) -> usize {
    let mut index = 0usize;

    for _ in 0..steps {
        index = next[index] as usize;
    }

    black_box(index)
}

#[hotpath::measure]
fn cache_sequential(next: &[u32]) -> usize {
    traverse(next, LEN)
}

#[hotpath::measure]
fn cache_random(next: &[u32]) -> usize {
    traverse(next, LEN)
}

#[hotpath::main]
fn main() {
    let sequential = sequential_chain();
    let random = random_chain();
    for _ in 0..ROUNDS {
        assert_eq!(cache_sequential(black_box(&sequential)), 0);
        assert_eq!(cache_random(black_box(&random)), 0);
    }
}
