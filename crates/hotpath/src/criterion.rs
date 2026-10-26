//! Criterion adapter (`criterion-0-8`): reuses existing Criterion benchmarks
//! to produce one deterministic hotpath report per benchmark.
//!
//! Swap the Criterion imports for these wrappers:
//!
//! ```ignore
//! use criterion::BenchmarkId;
//! use hotpath::criterion::{criterion_group, criterion_main, Criterion};
//! ```
//!
//! Without `HOTPATH_CRITERION_ITERATIONS` every wrapper forwards to Criterion
//! unchanged. With it set to N, the first benchmark routine entered starts a
//! [`HotpathGuard`](crate::HotpathGuard), runs the routine exactly N times
//! (each call measured as `hotpath::criterion::routine`) inside a single
//! Criterion iteration and drops the guard, which writes the report. That
//! needs Criterion's test mode (`--test`, which `cargo test --benches` and
//! `cargo run` imply) and a filter selecting one benchmark, e.g.
//! `cargo bench --bench codec -- --test --exact "group/id"`: entering a second
//! routine in the same process panics, since that means either Criterion's
//! measurement loop is running or the filter matched several benchmarks.
//!
//! Supported: [`Criterion::bench_function`], [`Criterion::bench_with_input`],
//! [`Criterion::benchmark_group`] and the group's counterparts, and
//! [`Bencher::iter`], [`Bencher::iter_batched`], [`Bencher::iter_batched_ref`],
//! [`Bencher::iter_custom`]. Anything else (`to_async`, `iter_with_setup`,
//! `iter_with_large_drop`, Criterion's own config builders on the wrapper) is
//! deliberately missing, so it fails to compile instead of silently running a
//! different number of iterations. The `config =` expression of
//! [`criterion_group!`](crate::criterion_group) is a plain
//! `criterion::Criterion`.

use std::collections::HashMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::Duration;

use ::criterion::measurement::{Measurement, WallTime};
use ::criterion::{BatchSize, BenchmarkId, PlotConfiguration, SamplingMode, Throughput};

#[doc(hidden)]
pub use ::criterion as __criterion;

/// Label of the report's wrapper row, covering everything between the start
/// of the routine loop and its end (setup and drops included).
const BENCH_LABEL: &str = "hotpath::criterion::bench";

/// `HOTPATH_CRITERION_ITERATIONS`: routine executions per benchmark. Blank or
/// unset leaves Criterion's own loop alone.
static ITERATIONS: LazyLock<Option<u64>> = LazyLock::new(|| {
    let raw = std::env::var_os("HOTPATH_CRITERION_ITERATIONS").filter(|v| !v.is_empty())?;
    let parsed = raw
        .to_str()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0);
    Some(parsed.unwrap_or_else(|| {
        panic!("HOTPATH_CRITERION_ITERATIONS: expected a positive integer, got {raw:?}")
    }))
});

/// Set once the fixed-iteration session ran; hotpath supports one guard per
/// process, so it never resets.
static SESSION_STARTED: AtomicBool = AtomicBool::new(false);

/// The fixed-iteration session of this process: its iteration count and the
/// guard whose drop writes the report.
struct Session {
    iterations: u64,
    _guard: crate::HotpathGuard,
}

impl Session {
    /// `None` without `HOTPATH_CRITERION_ITERATIONS`.
    fn start() -> Option<Session> {
        let iterations = (*ITERATIONS)?;
        assert!(
            !SESSION_STARTED.swap(true, Ordering::Relaxed),
            "HOTPATH_CRITERION_ITERATIONS runs one benchmark routine per process, but a second \
             one started: run Criterion in test mode with a filter matching one benchmark, \
             e.g. `cargo bench --bench <name> -- --test --exact <group/id>`"
        );
        let metadata =
            HashMap::from([("criterion_iterations".to_string(), iterations.to_string())]);
        let guard = crate::HotpathGuardBuilder::new(BENCH_LABEL)
            .user_metadata(metadata)
            .build();
        Some(Session {
            iterations,
            _guard: guard,
        })
    }
}

/// One benchmark routine execution, the report's per-iteration row.
#[hotpath::measure]
fn routine<O>(f: impl FnOnce() -> O) -> O {
    f()
}

#[doc(hidden)]
pub fn __finish<M: Measurement>(criterion: Criterion<M>) {
    criterion.inner.final_summary();
    if ITERATIONS.is_some() && !SESSION_STARTED.load(Ordering::Relaxed) {
        eprintln!(
            "HOTPATH_CRITERION_ITERATIONS is set but no benchmark ran, so no report was written: \
             check the benchmark filter (`--list` prints the ids)"
        );
        std::process::exit(1);
    }
}

/// [`criterion::Criterion`] whose benchmarks honor `HOTPATH_CRITERION_ITERATIONS`.
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
        BenchmarkGroup {
            inner: self.inner.benchmark_group(group_name),
        }
    }

    /// See [`criterion::Criterion::bench_function`].
    pub fn bench_function<F>(&mut self, id: &str, mut f: F) -> &mut Self
    where
        F: FnMut(&mut Bencher<'_, '_, M>),
    {
        self.inner
            .bench_function(id, |b| f(&mut Bencher { inner: b }));
        self
    }

    /// See [`criterion::Criterion::bench_with_input`].
    pub fn bench_with_input<F, I>(&mut self, id: BenchmarkId, input: &I, mut f: F) -> &mut Self
    where
        F: FnMut(&mut Bencher<'_, '_, M>, &I),
    {
        self.inner
            .bench_with_input(id, input, |b, i| f(&mut Bencher { inner: b }, i));
        self
    }
}

/// A benchmark id inside a group: a function name or a [`BenchmarkId`].
/// Criterion's own conversion trait is not nameable outside Criterion, so
/// this one forwards each kind to it as is.
pub trait IntoBenchmarkId {
    #[doc(hidden)]
    fn into_id(self) -> GroupId;
}

#[doc(hidden)]
pub enum GroupId {
    Name(String),
    Id(BenchmarkId),
}

impl IntoBenchmarkId for BenchmarkId {
    fn into_id(self) -> GroupId {
        GroupId::Id(self)
    }
}

impl IntoBenchmarkId for &str {
    fn into_id(self) -> GroupId {
        GroupId::Name(self.to_string())
    }
}

impl IntoBenchmarkId for String {
    fn into_id(self) -> GroupId {
        GroupId::Name(self)
    }
}

impl IntoBenchmarkId for &String {
    fn into_id(self) -> GroupId {
        GroupId::Name(self.clone())
    }
}

/// [`criterion::BenchmarkGroup`] whose benchmarks honor `HOTPATH_CRITERION_ITERATIONS`.
pub struct BenchmarkGroup<'a, M: Measurement = WallTime> {
    inner: ::criterion::BenchmarkGroup<'a, M>,
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
        let f = |b: &mut ::criterion::Bencher<'_, M>| f(&mut Bencher { inner: b });
        match id.into_id() {
            GroupId::Name(name) => self.inner.bench_function(name, f),
            GroupId::Id(id) => self.inner.bench_function(id, f),
        };
        self
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
        let f = |b: &mut ::criterion::Bencher<'_, M>, i: &I| f(&mut Bencher { inner: b }, i);
        match id.into_id() {
            GroupId::Name(name) => self.inner.bench_with_input(name, input, f),
            GroupId::Id(id) => self.inner.bench_with_input(id, input, f),
        };
        self
    }

    /// See [`criterion::BenchmarkGroup::finish`].
    pub fn finish(self) {
        self.inner.finish();
    }
}

/// [`criterion::Bencher`] that runs a routine exactly
/// `HOTPATH_CRITERION_ITERATIONS` times when it is set.
pub struct Bencher<'a, 'b, M: Measurement = WallTime> {
    inner: &'b mut ::criterion::Bencher<'a, M>,
}

impl<M: Measurement> Bencher<'_, '_, M> {
    /// See [`criterion::Bencher::iter`]. Like Criterion, each output is
    /// dropped inside the measured call.
    pub fn iter<O, R>(&mut self, mut routine: R)
    where
        R: FnMut() -> O,
    {
        let Some(session) = Session::start() else {
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

    /// See [`criterion::Bencher::iter_batched`]. In fixed-iteration mode
    /// every iteration runs `setup` right before `routine`, like
    /// [`BatchSize::PerIteration`], whatever `size` says: inputs are created,
    /// and outputs dropped, outside the measured call.
    pub fn iter_batched<I, O, S, R>(&mut self, mut setup: S, mut routine: R, size: BatchSize)
    where
        S: FnMut() -> I,
        R: FnMut(I) -> O,
    {
        let Some(session) = Session::start() else {
            return self.inner.iter_batched(setup, routine, size);
        };
        self.inner.iter(|| {
            for _ in 0..session.iterations {
                let input = black_box(setup());
                let output = crate::criterion::routine(|| routine(input));
                drop(black_box(output));
            }
        });
    }

    /// See [`criterion::Bencher::iter_batched_ref`]. Batching as in
    /// [`Bencher::iter_batched`]; outputs, then inputs, are dropped outside the
    /// measured call.
    pub fn iter_batched_ref<I, O, S, R>(&mut self, mut setup: S, mut routine: R, size: BatchSize)
    where
        S: FnMut() -> I,
        R: FnMut(&mut I) -> O,
    {
        let Some(session) = Session::start() else {
            return self.inner.iter_batched_ref(setup, routine, size);
        };
        self.inner.iter(|| {
            for _ in 0..session.iterations {
                let mut input = black_box(setup());
                let output = crate::criterion::routine(|| routine(&mut input));
                drop(black_box(output));
                drop(black_box(input));
            }
        });
    }

    /// See [`criterion::Bencher::iter_custom`]. In fixed-iteration mode
    /// `routine` is called once with the iteration count and runs its own
    /// loop, so the report has no per-iteration row.
    pub fn iter_custom<R>(&mut self, mut routine: R)
    where
        R: FnMut(u64) -> M::Value,
    {
        let Some(session) = Session::start() else {
            return self.inner.iter_custom(routine);
        };
        self.inner.iter_custom(|_| routine(session.iterations));
    }
}

/// [`criterion::criterion_group!`] passing [`Criterion`] to the targets. The
/// `config` expression is a `criterion::Criterion`.
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
/// [`CountingAllocator`](crate::CountingAllocator) as the global allocator,
/// like `#[hotpath::main]`, and exits with status 1 when
/// `HOTPATH_CRITERION_ITERATIONS` is set but no benchmark ran.
///
/// `allocator = Path;` before the groups wraps a custom unit-struct
/// allocator instead (replacing the benchmark's own `#[global_allocator]`);
/// `allocator = none;` installs nothing, for a benchmarked crate that already
/// declares `CountingAllocator` or `#[hotpath::main]` itself.
#[macro_export]
macro_rules! criterion_main {
    (allocator = none; $( $group:path ),+ $(,)* ) => {
        $crate::__criterion_main_fn!($( $group ),+);
    };
    (allocator = $alloc:path; $( $group:path ),+ $(,)* ) => {
        #[global_allocator]
        static __HOTPATH_CRITERION_ALLOCATOR: $crate::CountingAllocator<$alloc> =
            $crate::CountingAllocator::with($alloc);

        $crate::__criterion_main_fn!($( $group ),+);
    };
    ( $( $group:path ),+ $(,)* ) => {
        #[global_allocator]
        static __HOTPATH_CRITERION_ALLOCATOR: $crate::CountingAllocator =
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
