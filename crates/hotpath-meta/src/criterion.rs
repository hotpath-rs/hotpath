//! Criterion adapter (`criterion-0-8`): one hotpath report per existing
//! Criterion benchmark.
//!
//! ```ignore
//! use hotpath_meta::criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
//! ```
//!
//! Without `HOTPATH_META_CRITERION_ITERATIONS` every wrapper forwards to Criterion.
//! With it set to N, the selected routine runs exactly N times inside a single
//! Criterion iteration. Needs Criterion's test mode and a filter matching one
//! benchmark, e.g. `cargo bench --bench codec -- --test --exact "group/id"`.
//!
//! `Bencher::to_async` takes any Criterion `AsyncExecutor`; each routine
//! future is awaited to completion before the next one starts.
//!
//! Unwrapped Criterion APIs (`iter_with_setup`, ...) fail to compile instead
//! of running a different number of iterations.

use std::collections::HashMap;
use std::future::Future;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::Duration;

use ::criterion::async_executor::AsyncExecutor;
use ::criterion::measurement::{Measurement, WallTime};

use crate::shared::{CRITERION_BENCHMARK_KEY, CRITERION_ITERATIONS_KEY};

#[doc(hidden)]
pub use ::criterion as __criterion;

// Criterion's own types the wrappers take or return, so benchmarks need no
// direct `criterion` imports (or version match) for them.
pub use ::criterion::{
    async_executor, measurement, AxisScale, BatchSize, PlotConfiguration, PlottingBackend,
    SamplingMode, Throughput,
};

/// The plain [`criterion::Criterion`] that configures a group, as in
/// `criterion_group!(name = benches; config = Config::default().sample_size(10); targets = ...)`.
pub type Config<M = WallTime> = ::criterion::Criterion<M>;

const BENCH_LABEL: &str = "hotpath_meta::criterion::bench";

/// `HOTPATH_META_CRITERION_ITERATIONS`: routine executions per benchmark.
static ITERATIONS: LazyLock<Option<u64>> = LazyLock::new(|| {
    let raw = std::env::var_os("HOTPATH_META_CRITERION_ITERATIONS").filter(|v| !v.is_empty())?;
    let parsed = raw
        .to_str()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0);
    Some(parsed.unwrap_or_else(|| {
        panic!("HOTPATH_META_CRITERION_ITERATIONS: expected a positive integer, got {raw:?}")
    }))
});

/// Never resets: hotpath supports one guard per process.
static SESSION_STARTED: AtomicBool = AtomicBool::new(false);

struct Session {
    iterations: u64,
    _guard: crate::HotpathGuard,
}

impl Session {
    fn start(id: &str) -> Option<Session> {
        let iterations = (*ITERATIONS)?;
        assert!(
            !SESSION_STARTED.swap(true, Ordering::Relaxed),
            "HOTPATH_META_CRITERION_ITERATIONS runs one benchmark routine per process, but a second \
             one started: run Criterion in test mode with a filter matching one benchmark, \
             e.g. `cargo bench --bench <name> -- --test --exact <group/id>`"
        );
        let metadata = HashMap::from([
            (CRITERION_BENCHMARK_KEY.to_string(), id.to_string()),
            (CRITERION_ITERATIONS_KEY.to_string(), iterations.to_string()),
        ]);
        let guard = crate::HotpathGuardBuilder::new(BENCH_LABEL)
            .user_metadata(metadata)
            .build();
        Some(Session {
            iterations,
            _guard: guard,
        })
    }
}

/// Criterion's private `BatchSize::iters_per_batch`.
fn batch_size(size: BatchSize, iters: u64) -> u64 {
    let batch = match size {
        BatchSize::SmallInput => iters.div_ceil(10),
        BatchSize::LargeInput => iters.div_ceil(1000),
        BatchSize::PerIteration => 1,
        BatchSize::NumBatches(batches) => iters.div_ceil(batches),
        BatchSize::NumIterations(size) => size,
        _ => panic!("{size:?} is not a valid BatchSize"),
    };
    assert!(batch != 0, "Batch size must not be zero.");
    batch
}

/// The id `--exact` matches.
fn full_id(group: &str, function: Option<&str>, parameter: Option<&str>) -> String {
    match (function, parameter) {
        (Some(function), Some(parameter)) => format!("{group}/{function}/{parameter}"),
        (Some(name), None) | (None, Some(name)) => format!("{group}/{name}"),
        (None, None) => group.to_string(),
    }
}

#[hotpath_meta::measure]
fn routine<O>(f: impl FnOnce() -> O) -> O {
    f()
}

/// [`routine`] for a routine future: callers pass an `async` block that also
/// creates the future, so its whole execution, `.await`s included, is measured.
#[hotpath_meta::measure]
async fn async_routine<F: Future>(f: F) -> F::Output {
    f.await
}

#[doc(hidden)]
pub fn __finish<M: Measurement>(criterion: Criterion<M>) {
    criterion.inner.final_summary();
    if ITERATIONS.is_some() && !SESSION_STARTED.load(Ordering::Relaxed) {
        eprintln!(
            "HOTPATH_META_CRITERION_ITERATIONS is set but no benchmark ran, so no report was written: \
             check the benchmark filter (`--list` prints the ids)"
        );
        std::process::exit(1);
    }
}

/// [`criterion::Criterion`] whose benchmarks honor `HOTPATH_META_CRITERION_ITERATIONS`.
pub struct Criterion<M: Measurement = WallTime> {
    inner: ::criterion::Criterion<M>,
}

impl<M: Measurement> From<::criterion::Criterion<M>> for Criterion<M> {
    fn from(inner: ::criterion::Criterion<M>) -> Self {
        Self { inner }
    }
}

impl<M: Measurement + 'static> Criterion<M> {
    /// See [`criterion::Criterion::benchmark_group`].
    pub fn benchmark_group<S: Into<String>>(&mut self, group_name: S) -> BenchmarkGroup<'_, M> {
        let name = group_name.into();
        BenchmarkGroup {
            inner: self.inner.benchmark_group(name.clone()),
            name,
        }
    }

    /// See [`criterion::Criterion::bench_function`].
    pub fn bench_function<F>(&mut self, id: &str, mut f: F) -> &mut Self
    where
        F: FnMut(&mut Bencher<'_, '_, M>),
    {
        self.inner
            .bench_function(id, |b| f(&mut Bencher { inner: b, id }));
        self
    }

    /// See [`criterion::Criterion::bench_with_input`].
    pub fn bench_with_input<F, I>(&mut self, id: BenchmarkId, input: &I, mut f: F) -> &mut Self
    where
        F: FnMut(&mut Bencher<'_, '_, M>, &I),
    {
        let group = id.function.clone().unwrap_or_default();
        let full = full_id(&group, None, id.parameter.as_deref());
        self.inner
            .bench_with_input(id.into_criterion(), input, |b, i| {
                f(
                    &mut Bencher {
                        inner: b,
                        id: &full,
                    },
                    i,
                )
            });
        self
    }
}

/// [`criterion::BenchmarkId`] with readable parts, so the report can name the
/// benchmark.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BenchmarkId {
    function: Option<String>,
    parameter: Option<String>,
}

impl BenchmarkId {
    /// See [`criterion::BenchmarkId::new`].
    pub fn new<S: Into<String>, P: std::fmt::Display>(function_name: S, parameter: P) -> Self {
        Self {
            function: Some(function_name.into()),
            parameter: Some(parameter.to_string()),
        }
    }

    /// See [`criterion::BenchmarkId::from_parameter`].
    pub fn from_parameter<P: std::fmt::Display>(parameter: P) -> Self {
        Self {
            function: None,
            parameter: Some(parameter.to_string()),
        }
    }

    fn into_criterion(self) -> ::criterion::BenchmarkId {
        match (self.function, self.parameter) {
            (Some(function), Some(parameter)) => ::criterion::BenchmarkId::new(function, parameter),
            (None, Some(parameter)) => ::criterion::BenchmarkId::from_parameter(parameter),
            (function, None) => unreachable!("BenchmarkId {function:?} without a parameter"),
        }
    }
}

/// A benchmark id inside a group: a function name or a [`BenchmarkId`].
pub trait IntoBenchmarkId {
    #[doc(hidden)]
    fn into_id(self) -> BenchmarkId;
}

impl IntoBenchmarkId for BenchmarkId {
    fn into_id(self) -> BenchmarkId {
        self
    }
}

impl IntoBenchmarkId for &str {
    fn into_id(self) -> BenchmarkId {
        self.to_string().into_id()
    }
}

impl IntoBenchmarkId for String {
    fn into_id(self) -> BenchmarkId {
        BenchmarkId {
            function: Some(self),
            parameter: None,
        }
    }
}

impl IntoBenchmarkId for &String {
    fn into_id(self) -> BenchmarkId {
        self.clone().into_id()
    }
}

/// [`criterion::BenchmarkGroup`] whose benchmarks honor `HOTPATH_META_CRITERION_ITERATIONS`.
pub struct BenchmarkGroup<'a, M: Measurement = WallTime> {
    inner: ::criterion::BenchmarkGroup<'a, M>,
    name: String,
}

impl<M: Measurement> BenchmarkGroup<'_, M> {
    /// See [`criterion::BenchmarkGroup::sample_size`].
    pub fn sample_size(&mut self, n: usize) -> &mut Self {
        self.inner.sample_size(n);
        self
    }

    /// See [`criterion::BenchmarkGroup::warm_up_time`].
    pub fn warm_up_time(&mut self, dur: Duration) -> &mut Self {
        self.inner.warm_up_time(dur);
        self
    }

    /// See [`criterion::BenchmarkGroup::measurement_time`].
    pub fn measurement_time(&mut self, dur: Duration) -> &mut Self {
        self.inner.measurement_time(dur);
        self
    }

    /// See [`criterion::BenchmarkGroup::nresamples`].
    pub fn nresamples(&mut self, n: usize) -> &mut Self {
        self.inner.nresamples(n);
        self
    }

    /// See [`criterion::BenchmarkGroup::noise_threshold`].
    pub fn noise_threshold(&mut self, threshold: f64) -> &mut Self {
        self.inner.noise_threshold(threshold);
        self
    }

    /// See [`criterion::BenchmarkGroup::confidence_level`].
    pub fn confidence_level(&mut self, cl: f64) -> &mut Self {
        self.inner.confidence_level(cl);
        self
    }

    /// See [`criterion::BenchmarkGroup::significance_level`].
    pub fn significance_level(&mut self, sl: f64) -> &mut Self {
        self.inner.significance_level(sl);
        self
    }

    /// See [`criterion::BenchmarkGroup::plot_config`].
    pub fn plot_config(&mut self, new_config: PlotConfiguration) -> &mut Self {
        self.inner.plot_config(new_config);
        self
    }

    /// See [`criterion::BenchmarkGroup::throughput`].
    pub fn throughput(&mut self, throughput: Throughput) -> &mut Self {
        self.inner.throughput(throughput);
        self
    }

    /// See [`criterion::BenchmarkGroup::sampling_mode`].
    pub fn sampling_mode(&mut self, new_mode: SamplingMode) -> &mut Self {
        self.inner.sampling_mode(new_mode);
        self
    }

    /// See [`criterion::BenchmarkGroup::bench_function`].
    pub fn bench_function<ID: IntoBenchmarkId, F>(&mut self, id: ID, mut f: F) -> &mut Self
    where
        F: FnMut(&mut Bencher<'_, '_, M>),
    {
        self.bench_with_input(id, &(), |b, _| f(b))
    }

    /// See [`criterion::BenchmarkGroup::bench_with_input`].
    pub fn bench_with_input<ID: IntoBenchmarkId, F, I>(
        &mut self,
        id: ID,
        input: &I,
        mut f: F,
    ) -> &mut Self
    where
        F: FnMut(&mut Bencher<'_, '_, M>, &I),
        I: ?Sized,
    {
        let id = id.into_id();
        let full = full_id(&self.name, id.function.as_deref(), id.parameter.as_deref());
        let f = |b: &mut ::criterion::Bencher<'_, M>, i: &I| {
            f(
                &mut Bencher {
                    inner: b,
                    id: &full,
                },
                i,
            )
        };
        match id {
            BenchmarkId {
                function: Some(function),
                parameter: None,
            } => self.inner.bench_with_input(function, input, f),
            id => self.inner.bench_with_input(id.into_criterion(), input, f),
        };
        self
    }

    /// See [`criterion::BenchmarkGroup::finish`].
    pub fn finish(self) {
        self.inner.finish();
    }
}

/// [`criterion::Bencher`] that runs a routine exactly
/// `HOTPATH_META_CRITERION_ITERATIONS` times when it is set.
pub struct Bencher<'a, 'b, M: Measurement = WallTime> {
    inner: &'b mut ::criterion::Bencher<'a, M>,
    id: &'b str,
}

impl<'a, M: Measurement> Bencher<'a, '_, M> {
    /// See [`criterion::Bencher::to_async`].
    pub fn to_async<A: AsyncExecutor>(&mut self, runner: A) -> AsyncBencher<'a, '_, A, M> {
        AsyncBencher {
            inner: self.inner,
            runner,
            id: self.id,
        }
    }

    /// See [`criterion::Bencher::iter`].
    pub fn iter<O, R>(&mut self, mut routine: R)
    where
        R: FnMut() -> O,
    {
        let Some(session) = Session::start(self.id) else {
            return self.inner.iter(routine);
        };
        self.inner.iter(|| {
            for _ in 0..session.iterations {
                crate::criterion::routine(|| {
                    black_box(routine());
                });
            }
        });
    }

    /// See [`criterion::Bencher::iter_batched`]. Batches like Criterion.
    pub fn iter_batched<I, O, S, R>(&mut self, mut setup: S, mut routine: R, size: BatchSize)
    where
        S: FnMut() -> I,
        R: FnMut(I) -> O,
    {
        let Some(session) = Session::start(self.id) else {
            return self.inner.iter_batched(setup, routine, size);
        };
        let iterations = session.iterations;
        let batch = batch_size(size, iterations);
        self.inner.iter(|| {
            if batch == 1 {
                for _ in 0..iterations {
                    let input = black_box(setup());
                    let output = crate::criterion::routine(|| routine(input));
                    drop(black_box(output));
                }
                return;
            }
            let mut done = 0;
            while done < iterations {
                let len = batch.min(iterations - done);
                let inputs = black_box((0..len).map(|_| setup()).collect::<Vec<_>>());
                let mut outputs = Vec::with_capacity(len as usize);
                outputs.extend(
                    inputs
                        .into_iter()
                        .map(|input| crate::criterion::routine(|| routine(input))),
                );
                black_box(outputs);
                done += len;
            }
        });
    }

    /// See [`criterion::Bencher::iter_batched_ref`]. Batches like Criterion.
    pub fn iter_batched_ref<I, O, S, R>(&mut self, mut setup: S, mut routine: R, size: BatchSize)
    where
        S: FnMut() -> I,
        R: FnMut(&mut I) -> O,
    {
        let Some(session) = Session::start(self.id) else {
            return self.inner.iter_batched_ref(setup, routine, size);
        };
        let iterations = session.iterations;
        let batch = batch_size(size, iterations);
        self.inner.iter(|| {
            if batch == 1 {
                for _ in 0..iterations {
                    let mut input = black_box(setup());
                    let output = crate::criterion::routine(|| routine(&mut input));
                    drop(black_box(output));
                    drop(black_box(input));
                }
                return;
            }
            let mut done = 0;
            while done < iterations {
                let len = batch.min(iterations - done);
                let mut inputs = black_box((0..len).map(|_| setup()).collect::<Vec<_>>());
                let mut outputs = Vec::with_capacity(len as usize);
                outputs.extend(
                    inputs
                        .iter_mut()
                        .map(|input| crate::criterion::routine(|| routine(input))),
                );
                black_box(outputs);
                done += len;
            }
        });
    }

    /// See [`criterion::Bencher::iter_custom`]. The report has no
    /// per-iteration row.
    pub fn iter_custom<R>(&mut self, mut routine: R)
    where
        R: FnMut(u64) -> M::Value,
    {
        let Some(session) = Session::start(self.id) else {
            return self.inner.iter_custom(routine);
        };
        self.inner.iter_custom(|_| routine(session.iterations));
    }
}

/// [`criterion::AsyncBencher`] that runs a routine future exactly
/// `HOTPATH_META_CRITERION_ITERATIONS` times when it is set, awaiting each one
/// before the next starts.
///
/// Its methods take `self`: forwarding to Criterion moves the executor into
/// [`criterion::Bencher::to_async`], and `AsyncExecutor` is not implemented
/// for references to every executor.
pub struct AsyncBencher<'a, 'b, A, M: Measurement = WallTime> {
    inner: &'b mut ::criterion::Bencher<'a, M>,
    runner: A,
    id: &'b str,
}

impl<A: AsyncExecutor, M: Measurement> AsyncBencher<'_, '_, A, M> {
    /// See [`criterion::AsyncBencher::iter`].
    pub fn iter<O, R, F>(self, mut routine: R)
    where
        R: FnMut() -> F,
        F: Future<Output = O>,
    {
        let Some(session) = Session::start(self.id) else {
            return self.inner.to_async(self.runner).iter(routine);
        };
        let runner = &self.runner;
        self.inner.iter(|| {
            runner.block_on(async {
                for _ in 0..session.iterations {
                    crate::criterion::async_routine(async {
                        black_box(routine().await);
                    })
                    .await;
                }
            })
        });
    }

    /// See [`criterion::AsyncBencher::iter_batched`]. Batches like Criterion.
    pub fn iter_batched<I, O, S, R, F>(self, mut setup: S, mut routine: R, size: BatchSize)
    where
        S: FnMut() -> I,
        R: FnMut(I) -> F,
        F: Future<Output = O>,
    {
        let Some(session) = Session::start(self.id) else {
            return self
                .inner
                .to_async(self.runner)
                .iter_batched(setup, routine, size);
        };
        let iterations = session.iterations;
        let batch = batch_size(size, iterations);
        let runner = &self.runner;
        self.inner.iter(|| {
            runner.block_on(async {
                if batch == 1 {
                    for _ in 0..iterations {
                        let input = black_box(setup());
                        let output =
                            crate::criterion::async_routine(async { routine(input).await }).await;
                        drop(black_box(output));
                    }
                    return;
                }
                let mut done = 0;
                while done < iterations {
                    let len = batch.min(iterations - done);
                    let inputs = black_box((0..len).map(|_| setup()).collect::<Vec<_>>());
                    let mut outputs = Vec::with_capacity(len as usize);
                    for input in inputs {
                        outputs.push(
                            crate::criterion::async_routine(async { routine(input).await }).await,
                        );
                    }
                    black_box(outputs);
                    done += len;
                }
            })
        });
    }

    /// See [`criterion::AsyncBencher::iter_batched_ref`]. Batches like the
    /// synchronous [`Bencher::iter_batched_ref`]: a batch's inputs drop after
    /// its outputs.
    pub fn iter_batched_ref<I, O, S, R, F>(self, mut setup: S, mut routine: R, size: BatchSize)
    where
        S: FnMut() -> I,
        R: FnMut(&mut I) -> F,
        F: Future<Output = O>,
    {
        let Some(session) = Session::start(self.id) else {
            return self
                .inner
                .to_async(self.runner)
                .iter_batched_ref(setup, routine, size);
        };
        let iterations = session.iterations;
        let batch = batch_size(size, iterations);
        let runner = &self.runner;
        self.inner.iter(|| {
            runner.block_on(async {
                if batch == 1 {
                    for _ in 0..iterations {
                        let mut input = black_box(setup());
                        let output =
                            crate::criterion::async_routine(async { routine(&mut input).await })
                                .await;
                        drop(black_box(output));
                        drop(black_box(input));
                    }
                    return;
                }
                let mut done = 0;
                while done < iterations {
                    let len = batch.min(iterations - done);
                    let mut inputs = black_box((0..len).map(|_| setup()).collect::<Vec<_>>());
                    let mut outputs = Vec::with_capacity(len as usize);
                    for input in inputs.iter_mut() {
                        outputs.push(
                            crate::criterion::async_routine(async { routine(input).await }).await,
                        );
                    }
                    black_box(outputs);
                    done += len;
                }
            })
        });
    }

    /// See [`criterion::AsyncBencher::iter_custom`]. The report has no
    /// per-iteration row.
    pub fn iter_custom<R, F>(self, mut routine: R)
    where
        R: FnMut(u64) -> F,
        F: Future<Output = M::Value>,
    {
        let Some(session) = Session::start(self.id) else {
            return self.inner.to_async(self.runner).iter_custom(routine);
        };
        self.inner
            .to_async(self.runner)
            .iter_custom(|_| routine(session.iterations));
    }
}

/// [`criterion::criterion_group!`] passing [`Criterion`] to the targets.
#[macro_export]
macro_rules! criterion_group {
    (name = $name:ident; config = $config:expr; targets = $( $target:path ),+ $(,)*) => {
        #[doc = "The function which runs the benchmarks."]
        pub fn $name() {
            let mut criterion: $crate::criterion::Criterion<_> =
                $crate::criterion::Criterion::from($config.configure_from_args());
            $(
                $target(&mut criterion);
            )+
        }
    };
    ($name:ident, $( $target:path ),+ $(,)*) => {
        $crate::criterion_group! {
            name = $name;
            config = $crate::criterion::__criterion::Criterion::default();
            targets = $( $target ),+
        }
    };
}

/// [`criterion::criterion_main!`] that also installs
/// [`CountingAllocator`](crate::CountingAllocator) as the global allocator.
///
/// `allocator = Path;` wraps a custom unit-struct allocator instead;
/// `allocator = none;` installs nothing.
#[macro_export]
macro_rules! criterion_main {
    (allocator = none; $( $group:path ),+ $(,)* ) => {
        $crate::__criterion_main_fn!($( $group ),+);
    };
    (allocator = $alloc:path; $( $group:path ),+ $(,)* ) => {
        #[global_allocator]
        static __HOTPATH_META_CRITERION_ALLOCATOR: $crate::CountingAllocator<$alloc> =
            $crate::CountingAllocator::with($alloc);

        $crate::__criterion_main_fn!($( $group ),+);
    };
    ( $( $group:path ),+ $(,)* ) => {
        #[global_allocator]
        static __HOTPATH_META_CRITERION_ALLOCATOR: $crate::CountingAllocator =
            $crate::CountingAllocator::new();

        $crate::__criterion_main_fn!($( $group ),+);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __criterion_main_fn {
    ( $( $group:path ),+ ) => {
        fn main() {
            $(
                $group();
            )+
            $crate::criterion::__finish($crate::criterion::Criterion::from(
                $crate::criterion::__criterion::Criterion::default().configure_from_args(),
            ));
        }
    };
}

pub use crate::{criterion_group, criterion_main};
