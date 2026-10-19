# Profiling overhead

hotpath-rs is designed to collect detailed performance data with minimal impact on the profiled programs. In this docs section I will present the current overhead numbers and ways to further reduce them.

## Zero overhead when disabled

All instrumentation is gated behind the `hotpath` Cargo feature. With the feature off, every macro compiles to a no-op and all library dependencies stay out of the build, so there is no compile-time or runtime cost. The benchmarks confirm it: running the function benchmark without the feature shows no difference between the "instrumented" and raw variants (a `-0.5 ns/op` delta, i.e. measurement noise).

The numbers below describe what you pay while profiling is **enabled**.

## Methodology

Each number comes from a dedicated `benchmark_*` example in the [repository](https://github.com/hotpath-rs/hotpath/blob/main/CONTRIBUTING.md#overhead-benchmarks). Every benchmark hammers a single instrumented codepath in a tight, uncontended loop and compares it against an uninstrumented baseline in the same run, so the delta isolates the per-operation instrumentation cost. Each operation also performs ~1 µs of spin work to emulate a realistic function body:

```bash
cargo run --example benchmark_noop --features hotpath --release

noop: 100000 calls per mode
  baseline (raw)    1041.4 ns/op
  instrumented      1079.9 ns/op  (+38.6 ns/op vs baseline)
```

Results below were measured on an Apple M2 Pro, 100,000 iterations per mode, `--release` build. Run-to-run noise is around ±10 ns/op, so treat these as approximate magnitudes rather than exact constants.

Two design choices keep the hot path cheap:

- **Lock-free event transport** - every subsystem (functions, channels, streams, futures, locks, SQL) records events into per-thread lock-free chunked SPSC queues. Recording an event is a plain slot store plus one `Release` publish - no mutex, no read-modify-write atomics. A background worker is the single consumer and sweeps all queues every 50 ms.
- **Fast clock** - durations are measured with a custom `Instant` (`mach_absolute_time` on macOS, [quanta](https://docs.rs/quanta) on Linux) instead of `std::time::Instant`. A single `now()` call costs **6.1 ns** vs **17.2 ns** for the std clock, and most measurements need two of them.

## Overhead by profiled type

| Profiled type | Macro | Overhead per operation |
|---|---|---|
| Functions | `#[hotpath::measure]` | ~40 ns per call |
| Mutexes | `mutex!` | 29-54 ns per lock cycle |
| RwLocks | `rw_lock!` | 34-66 ns per acquisition |
| Channels | `channel!` | 47-88 ns per send/recv cycle |
| Futures | `future!` | ~125 ns per poll |
| Streams | `stream!` | ~30 ns per item |

### Functions

`#[hotpath::measure]` adds **~40 ns per call**: two clock reads bracketing the call, plus pushing one event into the thread-local queue. If a measured function runs for 4 µs, that is ~1% overhead; for functions in the tens-of-microseconds range and above, it disappears into noise. Measuring sub-microsecond functions in a hot loop is where the relative cost is highest - prefer instrumenting meaningful units of work over one-liner getters.

### Mutexes and RwLocks

`mutex!` and `rw_lock!` wrappers add **29-66 ns per lock cycle** (acquire + release), measured uncontended:

| Lock | Overhead per cycle |
|---|---|
| `parking_lot::Mutex` | +29 ns |
| `tokio::sync::Mutex` | +33 ns |
| `std::sync::Mutex` | +54 ns |
| `async_lock::Mutex` | +54 ns |
| `tokio::sync::RwLock` | +34 ns (read) / +38 ns (write) |
| `parking_lot::RwLock` | +49 ns (read) / +50 ns (write) |
| `std::sync::RwLock` | +59 ns (read) / +66 ns (write) |
| `async_lock::RwLock` | +56 ns (read) / +37 ns (write) |

Each cycle records two measurements (wait time and hold time), so this is roughly two function-measurement equivalents.

### Channels

The [channel wrappers](./data_flow.md) intercept `send`/`recv` inline and add **47-88 ns per send/recv cycle**:

| Channel | Overhead |
|---|---|
| `crossbeam_channel` | +47 ns |
| `futures_channel::mpsc` | +52 ns |
| `std::sync::mpsc` | +58 ns |
| `async_channel` | +63 ns |
| `flume` | +86 ns |
| `tokio::sync::mpsc` | +88 ns |

### Futures and streams

`future!` adds **~125 ns per poll** - it times every poll individually (two clock reads per poll) and tracks the future's lifecycle. `stream!` adds **~30 ns per yielded item**, since it only counts items and tracks stream state without timing each one.

## Reducing overhead: time sampling

For most applications the numbers above are negligible. But if you instrument something extremely hot - a function called millions of times per second, a busy channel - the clock reads become the dominant cost. **Time sampling** addresses exactly that.

Set a sampling rate in `[0.0, 1.0]` to measure durations for only a fraction of calls:

```bash
# time 1 in 10 calls
HOTPATH_TIME_SAMPLING_RATE=0.1 cargo run --features hotpath
```

or per resource type (these override the global rate):

```bash
HOTPATH_FUNCTIONS_TIME_SAMPLING_RATE=0.01
HOTPATH_MUTEXES_TIME_SAMPLING_RATE=0.1
HOTPATH_RW_LOCKS_TIME_SAMPLING_RATE=0.1
HOTPATH_FUTURES_TIME_SAMPLING_RATE=0.1
HOTPATH_CHANNELS_TIME_SAMPLING_RATE=0.1
```

The same config is available programmatically via [`HotpathGuardBuilder`](https://docs.rs/hotpath/latest/hotpath/struct.HotpathGuardBuilder.html) (`time_sampling_rate` plus per-resource setters); env vars take precedence.

For a skipped call, the sampler cuts the **two `Instant::now()` calls** that bracket each measurement - the biggest single cost on the hot path. Locks skip both the wait and hold stamp pairs, and wrap channels skip the send/receive latency stamps for unsampled messages. What you keep and what you lose:

- **Counts stay exact** - call counts, sent/received counts, queue sizes, and states are still recorded for every operation; only durations are sampled.
- **Durations become statistical** - avg and percentiles are computed from the sampled subset. Each call is timed independently at random with probability equal to the rate (e.g. `0.1` keeps about 1 in 10 calls), so periodic workloads cannot line up with a fixed sampling pattern, and the report header shows the active rates.
- **`0.0` is count-only mode** - no durations at all, and the timing clock reads are skipped entirely.

Sampling rates like `0.01`-`0.1` retain statistically useful latency distributions for high-frequency operations while removing most of the measurement cost. For low-frequency operations there is no reason to sample - the default (measure everything) gives exact numbers for free.
