//! Criterion benchmarks run through `hotpath::criterion`, one per Bencher
//! API. Each benchmark counts its routine calls, setups and drops on its
//! own and prints them to stderr once it finishes, so tests can check the
//! iteration count independently of the report. Allocation sizes are
//! distinct per site, so a report's exclusive bytes show where each one was
//! attributed.
//!
//! Run with:
//!   HOTPATH_CRITERION_ITERATIONS=1000 cargo run -p test-criterion --example fixture --features hotpath,hotpath-alloc -- --exact work/iter_batched

use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use criterion::{BatchSize, BenchmarkId};
use hotpath::criterion::{criterion_group, criterion_main, Criterion};

/// Allocated by `work`, the instrumented library function.
const WORK_BYTES: usize = 512;
/// Allocated by the benchmark routine itself.
const ROUTINE_BYTES: usize = 64;
/// Allocated by each `setup`.
const SETUP_BYTES: usize = 1024;
/// Allocated while an `Output` drops.
const OUTPUT_DROP_BYTES: usize = 32;
/// Allocated while an `Input` drops.
const INPUT_DROP_BYTES: usize = 16;

static ROUTINE_CALLS: AtomicU64 = AtomicU64::new(0);
static SETUP_CALLS: AtomicU64 = AtomicU64::new(0);
static INPUT_DROPS: AtomicU64 = AtomicU64::new(0);
static OUTPUT_DROPS: AtomicU64 = AtomicU64::new(0);

#[hotpath::measure]
#[inline(never)]
fn work(seed: u64) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(WORK_BYTES);
    for i in 0..WORK_BYTES as u64 {
        bytes.push(seed.wrapping_mul(31).wrapping_add(black_box(i)) as u8);
    }
    bytes
}

struct Input {
    seed: u64,
    _buffer: Vec<u8>,
}

impl Input {
    fn setup() -> Self {
        SETUP_CALLS.fetch_add(1, Ordering::Relaxed);
        Self {
            seed: 7,
            _buffer: black_box(Vec::with_capacity(SETUP_BYTES)),
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        INPUT_DROPS.fetch_add(1, Ordering::Relaxed);
        black_box(Vec::<u8>::with_capacity(INPUT_DROP_BYTES));
    }
}

struct Output {
    _work: Vec<u8>,
    _tag: Vec<u8>,
}

impl Drop for Output {
    fn drop(&mut self) {
        OUTPUT_DROPS.fetch_add(1, Ordering::Relaxed);
        black_box(Vec::<u8>::with_capacity(OUTPUT_DROP_BYTES));
    }
}

/// The measured routine body shared by every benchmark.
fn run(seed: u64) -> Output {
    ROUTINE_CALLS.fetch_add(1, Ordering::Relaxed);
    Output {
        _work: work(seed),
        _tag: black_box(Vec::with_capacity(ROUTINE_BYTES)),
    }
}

fn print_counts() {
    eprintln!(
        "fixture: routine_calls={} setup_calls={} input_drops={} output_drops={}",
        ROUTINE_CALLS.load(Ordering::Relaxed),
        SETUP_CALLS.load(Ordering::Relaxed),
        INPUT_DROPS.load(Ordering::Relaxed),
        OUTPUT_DROPS.load(Ordering::Relaxed),
    );
}

fn bench_top_level(c: &mut Criterion) {
    c.bench_function("top_level", |b| {
        b.iter(|| run(1));
        print_counts();
    });
}

fn bench_work(c: &mut Criterion) {
    let mut group = c.benchmark_group("work");
    group.bench_function("iter", |b| {
        b.iter(|| run(1));
        print_counts();
    });
    group.bench_function("iter_batched", |b| {
        b.iter_batched(Input::setup, |input| run(input.seed), BatchSize::SmallInput);
        print_counts();
    });
    group.bench_function("iter_batched_ref", |b| {
        b.iter_batched_ref(
            Input::setup,
            |input: &mut Input| run(input.seed),
            BatchSize::LargeInput,
        );
        print_counts();
    });
    group.bench_function("iter_custom", |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                black_box(run(1));
            }
            start.elapsed()
        });
        print_counts();
    });
    group.finish();
}

fn bench_inputs(c: &mut Criterion) {
    let mut group = c.benchmark_group("inputs");
    for seed in [3u64, 5] {
        group.bench_with_input(BenchmarkId::new("work", seed), &seed, |b, &seed| {
            b.iter(|| run(seed));
            print_counts();
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = criterion::Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(50))
        .measurement_time(Duration::from_millis(200));
    targets = bench_top_level, bench_work, bench_inputs
}
criterion_main!(benches);
