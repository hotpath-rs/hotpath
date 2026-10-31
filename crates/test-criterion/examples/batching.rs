//! Batched Criterion benchmarks that log every setup (`S`), routine (`R`),
//! output drop (`O`) and input drop (`I`), printed to stderr after each
//! benchmark, so tests can check batch ordering and partial last batches.
//! The `async_batched` group runs on a custom executor and logs each routine
//! after an `.await` that returns `Pending` once.
//!
//! Run with:
//!   HOTPATH_CRITERION_ITERATIONS=25 cargo run -p test-criterion --example batching --features hotpath -- --exact batched/value/10

use std::future::Future;
use std::pin::{pin, Pin};
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

use hotpath::criterion::async_executor::AsyncExecutor;
use hotpath::criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};

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

/// Busy-polls a future to completion.
struct SpinExecutor;

impl AsyncExecutor for SpinExecutor {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        let mut future = pin!(future);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
        }
    }
}

/// Returns `Pending` on its first poll.
struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if std::mem::replace(&mut self.0, true) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
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

fn bench_async_batched(c: &mut Criterion) {
    let mut group = c.benchmark_group("async_batched");
    for size in [1u64, 10] {
        group.bench_function(BenchmarkId::new("value", size), |b| {
            b.to_async(SpinExecutor).iter_batched(
                setup,
                |input| async move {
                    YieldOnce(false).await;
                    log('R');
                    let _input = input;
                    Output
                },
                BatchSize::NumIterations(size),
            );
            print_trace();
        });
        group.bench_function(BenchmarkId::new("ref", size), |b| {
            b.to_async(SpinExecutor).iter_batched_ref(
                setup,
                |_input: &mut Input| async {
                    YieldOnce(false).await;
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

criterion_group!(benches, bench_batched, bench_async_batched);
criterion_main!(benches);
