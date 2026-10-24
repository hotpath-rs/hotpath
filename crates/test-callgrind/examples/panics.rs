//! A measured workload that panics before the guard drops: a requested
//! merge must be skipped, never write a report built from the partial run.
//!
//! Run with:
//!   HOTPATH_OUTPUT_FORMAT=json HOTPATH_MERGE_REPORT=native.json cargo run --release -p test-callgrind --example panics --features hotpath,hotpath-callgrind,hotpath-cloud

#[hotpath::measure]
fn work() -> u64 {
    (0..1_000u64).sum()
}

#[hotpath::main]
fn main() {
    std::hint::black_box(work());
    panic!("workload failed");
}
