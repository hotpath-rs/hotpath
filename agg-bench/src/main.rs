//! Deterministic allocation workload for the aggregate PR comment tests.
//!
//! Run with:
//! AGG_BYTES=4096 AGG_CALLS=100 cargo run --release --manifest-path agg-bench/Cargo.toml --features hotpath-alloc

fn env_usize(name: &str) -> usize {
    let raw = std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set"));
    raw.parse()
        .unwrap_or_else(|_| panic!("{name} must be a non-negative integer, got {raw:?}"))
}

#[hotpath::measure]
fn allocate(bytes: usize) -> usize {
    let buffer = vec![1u8; bytes];
    std::hint::black_box(&buffer).len()
}

#[hotpath::main]
fn main() {
    let bytes = env_usize("AGG_BYTES");
    let calls = env_usize("AGG_CALLS");
    let mut total = 0;
    for _ in 0..calls {
        total += allocate(bytes);
    }
    std::hint::black_box(total);
}
