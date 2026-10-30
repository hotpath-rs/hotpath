//! Criterion benchmarks run through `hotpath::criterion`, one per Bencher
//! and AsyncBencher API. Each prints its own routine, setup and drop counts to stderr, and
//! every allocation site uses a distinct size.
//!
//! Run with:
//!   HOTPATH_CRITERION_ITERATIONS=1000 cargo run -p test-criterion --example fixture --features hotpath,hotpath-alloc -- --exact work/iter_batched_per_iteration

use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use criterion::BatchSize;
use hotpath::criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

const WORK_BYTES: usize = 512;
const ROUTINE_BYTES: usize = 64;
const SETUP_BYTES: usize = 1024;
const OUTPUT_DROP_BYTES: usize = 32;
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

fn run(seed: u64) -> Output {
    ROUTINE_CALLS.fetch_add(1, Ordering::Relaxed);
    Output {
        _work: work(seed),
        _tag: black_box(Vec::with_capacity(ROUTINE_BYTES)),
    }
}

/// [`run`] after yielding to the executor, so the measured work happens
/// after an `.await`.
async fn run_async(seed: u64) -> Output {
    tokio::task::yield_now().await;
    run(seed)
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
    c.bench_with_input(BenchmarkId::new("top_input", 9), &9u64, |b, &seed| {
        b.iter(|| run(seed));
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
    group.bench_function("iter_batched_per_iteration", |b| {
        b.iter_batched(
            Input::setup,
            |input| run(input.seed),
            BatchSize::PerIteration,
        );
        print_counts();
    });
    group.bench_function("iter_batched_ref_per_iteration", |b| {
        b.iter_batched_ref(
            Input::setup,
            |input: &mut Input| run(input.seed),
            BatchSize::PerIteration,
        );
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

fn bench_async(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let multi_thread = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()
        .unwrap();
    let mut group = c.benchmark_group("async");
    group.bench_function("iter", |b| {
        b.to_async(&rt).iter(|| run_async(1));
        print_counts();
    });
    group.bench_function("iter_multi_thread", |b| {
        b.to_async(&multi_thread).iter(|| run_async(1));
        print_counts();
    });
    group.bench_function("iter_batched", |b| {
        b.to_async(&rt).iter_batched(
            Input::setup,
            |input| async move { run_async(input.seed).await },
            BatchSize::SmallInput,
        );
        print_counts();
    });
    group.bench_function("iter_batched_per_iteration", |b| {
        b.to_async(&rt).iter_batched(
            Input::setup,
            |input| async move { run_async(input.seed).await },
            BatchSize::PerIteration,
        );
        print_counts();
    });
    group.bench_function("iter_batched_ref_per_iteration", |b| {
        b.to_async(&rt).iter_batched_ref(
            Input::setup,
            |input: &mut Input| run_async(input.seed),
            BatchSize::PerIteration,
        );
        print_counts();
    });
    group.bench_function("iter_batched_ref", |b| {
        b.to_async(&rt).iter_batched_ref(
            Input::setup,
            |input: &mut Input| run_async(input.seed),
            BatchSize::LargeInput,
        );
        print_counts();
    });
    group.bench_function("iter_custom", |b| {
        b.to_async(&rt).iter_custom(|iters| async move {
            let start = Instant::now();
            for _ in 0..iters {
                black_box(run_async(1).await);
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
    group.bench_function(BenchmarkId::from_parameter("seven"), |b| {
        b.iter(|| run(7));
        print_counts();
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = criterion::Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_millis(50))
        .measurement_time(Duration::from_millis(200));
    targets = bench_top_level, bench_work, bench_async, bench_inputs
}
criterion_main!(benches);
