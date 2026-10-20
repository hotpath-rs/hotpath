//! Two profiling sessions in one process, run sequentially: under Callgrind
//! each session's guard writes its own dump, so the second report counts
//! only the second session's calls. Reports go to the paths given as args.
//!
//! Run with:
//!   CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" cargo run --release -p test-callgrind --example sessions --features hotpath,hotpath-callgrind -- first.json second.json

use std::hint::black_box;

#[inline(never)]
fn spin(n: u64) -> u64 {
    let mut acc = 0u64;
    for i in 0..n {
        acc = acc.wrapping_mul(31).wrapping_add(black_box(i));
    }
    acc
}

#[hotpath::measure]
fn work() -> u64 {
    spin(100_000)
}

fn session(output_path: &str, calls: usize) {
    let _guard = hotpath::HotpathGuardBuilder::new("session")
        .format(hotpath::Format::Json)
        .output_path(output_path)
        .build();
    for _ in 0..calls {
        black_box(work());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    session(&args[0], 1);
    session(&args[1], 3);
}
