//! The `drain-criterion` hotpath-cloud benchmark: the hotpath-drain lockstep
//! scenario as a Criterion benchmark, profiled by `hotpath-meta`.
//!
//! Run with:
//!   cargo bench -p test-drain-criterion --bench drain
//!   HOTPATH_META_CRITERION_ITERATIONS=5 cargo bench -p test-drain-criterion --bench drain --features hotpath-alloc-meta -- --test --exact drain/lockstep
//!   CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" HOTPATH_META_CRITERION_ITERATIONS=5 cargo bench -p test-drain-criterion --bench drain --features hotpath-alloc-meta,hotpath-callgrind-meta -- --test --exact drain/lockstep

#[path = "../../hotpath-drain/examples/support/lockstep.rs"]
mod lockstep;

use hotpath_meta::criterion::{criterion_group, criterion_main, Criterion};

fn bench_drain(c: &mut Criterion) {
    let mut group = c.benchmark_group("drain");
    group.sample_size(10);
    group.bench_function("lockstep", |b| b.iter(lockstep::run));
    group.finish();
}

criterion_group!(benches, bench_drain);
criterion_main!(benches);
