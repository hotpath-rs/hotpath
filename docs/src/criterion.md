# Criterion benchmarks

Existing [Criterion.rs](https://crates.io/crates/criterion) benchmarks can produce hotpath reports, with function timing, allocations and [Callgrind instruction counts](callgrind.md), without rewriting them. Each run executes one benchmark a **fixed number of times** and writes one report, so native and Callgrind runs of the same build do identical work and can be [merged](callgrind.md#timing-and-instruction-counts-in-one-report) and uploaded to [hotpath-cloud](cloud.md).

Supported version: Criterion `0.8` (the `criterion-0-8` feature).

## Setup

Enable the `criterion-0-8` feature and forward hotpath's profiling features:

```toml
[dependencies]
hotpath = { version = "{{HOTPATH_VERSION}}", features = ["criterion-0-8"] }

[dev-dependencies]
criterion = "0.8"

[features]
hotpath = ["hotpath/hotpath"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
hotpath-callgrind = ["hotpath", "hotpath/hotpath-callgrind"]
hotpath-cloud = ["hotpath", "hotpath/hotpath-cloud"]
```

In each benchmark file, import `Criterion`, `BenchmarkId`, `criterion_group!` and `criterion_main!` from `hotpath::criterion` instead of `criterion`. Everything else (`BatchSize`, `Throughput`, ...) still comes from `criterion`. hotpath's `BenchmarkId` has the same constructors, and lets the report name the benchmark it ran:

```rust
use criterion::BatchSize;
use hotpath::criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

fn bench_codec(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec");
    group.bench_function("encode", |b| b.iter(|| encode(&VALUES)));
    group.bench_function("decode", |b| {
        b.iter_batched(|| encode(&VALUES), |bytes| decode(&bytes), BatchSize::SmallInput)
    });
    group.finish();
}

criterion_group!(benches, bench_codec);
criterion_main!(benches);
```

The `config = ...` form of `criterion_group!` takes a plain `criterion::Criterion`, so write `config = criterion::Criterion::default().sample_size(10)`.

`criterion_main!` installs hotpath's tracking allocator, as `#[hotpath::main]` does. If the benchmark declares its own `#[global_allocator]`, remove it and pass the allocator instead: `criterion_main!(allocator = tikv_jemallocator::Jemalloc; benches)`. If the benchmarked library already installs one (it uses `#[hotpath::main]` or declares `hotpath::CountingAllocator`), pass `allocator = none;`.

Without `HOTPATH_CRITERION_ITERATIONS` the wrappers forward to Criterion unchanged: `cargo bench` and `cargo test --benches` behave exactly as before. Build those runs without hotpath's profiling features: with them on, measured functions and the tracking allocator still do their bookkeeping when no report is being written, which biases Criterion's own timings. Without them the wrappers and `CountingAllocator` are plain pass-throughs.

## Running one benchmark

Set `HOTPATH_CRITERION_ITERATIONS` and select one benchmark in Criterion's test mode:

```bash
HOTPATH_CRITERION_ITERATIONS=1000 \
  cargo bench --bench codec --features hotpath-alloc -- --test --exact codec/decode
```

The routine then runs exactly 1,000 times inside a single Criterion iteration, and the report has:

- `hotpath::criterion::routine`: one call per routine execution, with its timing and exclusive allocations.
- Every `#[hotpath::measure]` function the routine calls, e.g. the library functions under benchmark.
- `hotpath::criterion::bench`: the whole loop, including setup and drops outside the routine.
- `criterion_benchmark` (the full id, e.g. `codec/decode`) and `criterion_iterations` in the report's `user_metadata`. A [merge](#native-and-callgrind-runs) refuses a native report whose values differ, or that has them while the Callgrind run has not (or the other way round).

`cargo bench -- --list` prints the benchmark ids (with `HOTPATH_CRITERION_ITERATIONS` unset, since a run that benchmarks nothing exits with status 1). Run each benchmark in its own process, with its own `HOTPATH_BENCHMARK` (and `HOTPATH_OUTPUT_PATH`, if writing files), so every Criterion benchmark is a separate hotpath-cloud benchmark series: a filter matching more than one benchmark, or Criterion's measurement mode (no `--test`), panics on the second routine instead of mixing benchmarks in one report. A filter matching nothing exits with status 1.

### What the routine row includes

| API | Measured in `routine` | Outside it, in `bench` |
|---|---|---|
| `iter` | the routine and dropping its output | - |
| `iter_batched` | the routine, including dropping the input it consumes | `setup`, dropping the output |
| `iter_batched_ref` | the routine | `setup`, dropping the output and the input |
| `iter_custom` | no routine row: it is called once with the iteration count and loops on its own | everything |

`iter_batched` and `iter_batched_ref` batch the N iterations like Criterion: each batch (sized from `BatchSize` and N, the last one holding what is left) runs every `setup` first, then the routines, then drops the outputs (and, for `iter_batched_ref`, the inputs). The Vecs holding a batch's inputs and outputs count towards the `bench` row. Setup allocations never count towards the routine or the functions it calls, but a `#[hotpath::measure]` function called by `setup` itself still gets those calls in its own row.

A fixed iteration count fixes how many times the routine runs, not what it does: random inputs, state that changes between iterations (caches, growing collections) and concurrency still change timings and instruction counts between runs.

### Unsupported

`to_async`, `iter_with_setup`, `iter_with_large_drop`, `iter_with_large_setup`, Criterion's own `BenchmarkId` and Criterion's configuration methods on the `Criterion` wrapper are not provided, so they fail to compile rather than silently running a different number of iterations or writing an unnamed report. Criterion's `Profiler` hooks are not used.

## Native and Callgrind runs

The same binary runs natively and under Callgrind. Callgrind attribution needs the symbol table, and benchmarks inherit the `release` profile, which projects often strip or build with fat LTO (slow to rebuild for every benchmark job). Add a dedicated profile:

```toml
[profile.hotpath]
inherits = "release"
lto = false
strip = "none"
```

Build once with it, then run the binary directly (Cargo prints its path):

```bash
cargo bench --profile hotpath --bench codec --features hotpath-alloc,hotpath-cloud,hotpath-callgrind --no-run
BIN=target/hotpath/deps/codec-<hash>

export HOTPATH_CRITERION_ITERATIONS=1000 HOTPATH_BENCHMARK=codec-decode HOTPATH_OUTPUT_FORMAT=json

HOTPATH_OUTPUT_PATH=native.json $BIN --bench --test --exact codec/decode

HOTPATH_OUTPUT_PATH=merged.json HOTPATH_MERGE_REPORT=native.json \
  valgrind -q --tool=callgrind --instr-atstart=no \
  $BIN --bench --test --exact codec/decode
```

`merged.json` holds the native timing and allocations next to the instruction counts of `hotpath::criterion::routine` and every `#[hotpath::measure]` function, ready for upload with `HOTPATH_UPLOAD`. `measure_block!` blocks get no instruction counts, so measure the functions you want counted with `#[hotpath::measure]`.

For a deterministic workload, a measured function that calls no other measured function has the same count in every run. `hotpath::criterion::routine`, and any function calling many measured functions, also counts hotpath's queueing of the nested calls' events, which varies by a small fraction of a percent with the timing of hotpath's background threads.
