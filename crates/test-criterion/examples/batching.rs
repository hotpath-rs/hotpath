//! Batched Criterion benchmarks that log every setup (`S`), routine (`R`),
//! output drop (`O`) and input drop (`I`), printed to stderr after each
//! benchmark, so tests can check batch ordering and partial last batches.
//!
//! Run with:
//!   HOTPATH_CRITERION_ITERATIONS=25 cargo run -p test-criterion --example batching --features hotpath -- --exact batched/value/10

use std::sync::Mutex;

use criterion::BatchSize;
use hotpath::criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

static TRACE: Mutex<String> = Mutex::new(String::new());

fn log(event: char) {
    TRACE.lock().unwrap().push(event);
}

struct Input;

impl Drop for Input {
    fn drop(&mut self) {
        log('I');
    }
}

struct Output;

impl Drop for Output {
    fn drop(&mut self) {
        log('O');
    }
}

fn setup() -> Input {
    log('S');
    Input
}

fn print_trace() {
    eprintln!("trace: {}", std::mem::take(&mut *TRACE.lock().unwrap()));
}

fn bench_batched(c: &mut Criterion) {
    let mut group = c.benchmark_group("batched");
    for size in [1u64, 10] {
        group.bench_function(BenchmarkId::new("value", size), |b| {
            b.iter_batched(
                setup,
                |input| {
                    // Consumes the input; it drops after the routine logs.
                    log('R');
                    let _input = input;
                    Output
                },
                BatchSize::NumIterations(size),
            );
            print_trace();
        });
        group.bench_function(BenchmarkId::new("ref", size), |b| {
            b.iter_batched_ref(
                setup,
                |_input: &mut Input| {
                    log('R');
                    Output
                },
                BatchSize::NumIterations(size),
            );
            print_trace();
        });
    }
    group.finish();
}

criterion_group!(benches, bench_batched);
criterion_main!(benches);
