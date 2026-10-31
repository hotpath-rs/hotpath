//! Sample use of hotpath-drain: one run of the lockstep scenario in
//! `support/lockstep.rs`.
//!
//! Run with:
//!   cargo run -p hotpath-drain --example throughput --release
//!   cargo run -p hotpath-drain --example throughput --release --features hotpath-meta
//!   cargo run -p hotpath-drain --example throughput --release --features hotpath-alloc-meta
//!   CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" cargo run -p hotpath-drain --example throughput --release --features hotpath-alloc-meta,hotpath-callgrind-meta

#[path = "support/lockstep.rs"]
mod lockstep;

use lockstep::{PRODUCERS, TOTAL_EVENTS};

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::main(percentiles = [95, 99, 99.9]))]
fn main() {
    // A 1 MB scratch buffer on the main thread: an allocation regression of
    // `throughput::main` that `[functions.alloc] ignore` leaves unjudged.
    std::hint::black_box(Vec::<u8>::with_capacity(1 << 20));
    let elapsed = lockstep::run();
    println!(
        "{PRODUCERS} producers sent {TOTAL_EVENTS} events, consumer received {TOTAL_EVENTS} in {elapsed:.2?} ({:.1} M events/s)",
        TOTAL_EVENTS as f64 / elapsed.as_secs_f64() / 1e6
    );
}
