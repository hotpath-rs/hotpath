//! The `fib-criterion-hetzner` hotpath-cloud benchmark: a single-threaded
//! recursive Fibonacci as a Criterion benchmark, profiled by `hotpath-meta`.
//! It has no cross-thread communication, so its run-to-run timing spread is a
//! baseline for the machine's own noise next to the multi-threaded `drain`
//! benchmark.
//!
//! Run with:
//!   cargo bench -p test-drain-criterion --bench fib
//!   HOTPATH_META_CRITERION_ITERATIONS=5 cargo bench -p test-drain-criterion --bench fib --features hotpath-alloc-meta -- --test --exact fib/recursive

use std::hint::black_box;

use hotpath_meta::criterion::{criterion_group, criterion_main, Criterion};

/// Sized for about one second per call on the Hetzner runner. Changing it
/// breaks comparability with previously uploaded reports.
const N: u64 = 42;

/// Not instrumented: a measurement per recursive call would dwarf the work.
fn fib(n: u64) -> u64 {
    if n < 2 {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

fn bench_fib(c: &mut Criterion) {
    let mut group = c.benchmark_group("fib");
    group.sample_size(10);
    group.bench_function("recursive", |b| b.iter(|| fib(black_box(N))));
    group.finish();
}

criterion_group!(benches, bench_fib);
criterion_main!(benches);
