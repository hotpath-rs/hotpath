//! Run with:
//!   cargo run --manifest-path crates/test-nested-workspace/Cargo.toml --example nested_workspace --features hotpath

#[hotpath::measure]
fn nested_function(n: u64) -> u64 {
    (0..n).sum()
}

#[hotpath::main]
fn main() {
    for i in 0..100 {
        std::hint::black_box(nested_function(i));
    }
}
