# Deterministic instruction counts with Callgrind

Wall-clock benchmarks are noisy, especially in GitHub Actions CI. CPU frequency scaling, scheduling, and shared runners can introduce significant variance, making small performance regressions difficult to detect.

`hotpath-rs` can count executed instructions instead of measuring elapsed time, using [Valgrind's Callgrind](https://valgrind.org/docs/manual/cl-manual.html).

Instruction counts are highly reproducible for deterministic workloads, making them useful for detecting performance regressions in CI. They don't require hardware performance counters, so they work on standard GitHub-hosted runners. Instruction counts are not a direct replacement for timings, although for most CPU-bound workloads the two are closely correlated. 

**Platform support:** x86_64 and aarch64 Linux. On other platforms, the feature compiles but falls back to the standard timing report. Instruction counts are comparable only within one architecture: the same code executes a different number of instructions on x86_64 and aarch64.

On aarch64, released Valgrind versions (up to at least 3.28.0) misattribute inclusive instruction counts: their arm64 decoder treats every plain `B` branch as a call, so Callgrind reports fake recursion and inflated totals. Use the patched build from [hotpath-rs/valgrind](https://github.com/hotpath-rs/valgrind/releases), an unofficial fork not affiliated with the Valgrind project, until the fix is upstream.

## Running under Callgrind

Install a pinned Valgrind version (`sudo apt-get install -y valgrind=1:3.26.0-0ubuntu1` on Ubuntu 26.04), then enable the `hotpath-callgrind` feature:

`Cargo.toml`
```toml
[features]
hotpath = ["hotpath/hotpath"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
hotpath-callgrind = ["hotpath", "hotpath/hotpath-callgrind"]
```

and run the program under Callgrind, for example through Cargo's target runner:

```bash
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" \
  cargo run --release --features=hotpath-callgrind
```

On aarch64 Linux, the variable is `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER`.

Under Valgrind, hotpath does not report timing, to avoid confusing it with real durations: Callgrind's instrumentation slows execution down substantially, potentially by tens of times or more, so measured times are meaningless. The `icount` (instruction count) report replaces the timing report:

```
icount - Inclusive instructions executed per function (Callgrind Ir).
Total: 39,730,412
+------------------------------+-------+-----------+------------+---------+
| Function                     | Calls | Avg       | Total      | % Total |
+------------------------------+-------+-----------+------------+---------+
| instructions::recursive      | 4     | 4,500,564 | 18,002,258 | 45.31%  |
+------------------------------+-------+-----------+------------+---------+
| instructions::async_fn       | 2     | 7,200,797 | 14,401,595 | 36.25%  |
+------------------------------+-------+-----------+------------+---------+
| instructions::free_fn        | 2     | 1,800,012 | 3,600,024  | 9.06%   |
+------------------------------+-------+-----------+------------+---------+
```

In the JSON report it is the `functions_icount` section; select it with `HOTPATH_REPORT=functions-icount` and judge it in a [policy file](./regression_policy.md) under `[functions.icount]`.

The same build run natively prints the usual timing report, so one binary serves both runs. Under Valgrind durations are not measured at all (every time sampling rate is forced to `0.0`), so even an explicit `HOTPATH_REPORT=functions-timing` contains no timings. To get timing and instruction counts in one report, merge a native run's report into the Callgrind run's, as described [below](#timing-and-instruction-counts-in-one-report).

Callgrind writes its counts to `callgrind.out.<pid>` files in the working directory. If you pass a custom `--callgrind-out-file`, set `HOTPATH_CALLGRIND_OUT_FILE` to the same value (`%p` and `%q{VAR}` are expanded like Valgrind does).

## Callgrind options

Callgrind has two separate switches, and two similarly named options set where they start:

- **Instrumentation** (`--instr-atstart`, default `yes`) decides whether Callgrind simulates the program at all. It is what makes Callgrind slow. hotpath turns it on at guard creation and off at guard drop.
- **Collection** (`--collect-atstart`, default `yes`) is a per-thread flag that decides whether a thread's instructions are added to the counts while instrumentation is on. Whenever instrumentation is switched on or off, Callgrind resets every thread's flag to the `--collect-atstart` value.

hotpath counts only what runs between guard creation and drop. At guard creation it discards everything counted so far (program startup, dynamic loading, guard setup). At guard drop it writes its dump and stops instrumenting, so report building and uploading are not counted either.

**`--instr-atstart=no` is optional and recommended.** It changes speed, not counts: the program runs uninstrumented, much faster than under full simulation, until the guard turns instrumentation on. Without it, Callgrind simulates startup in full and hotpath discards those counts at guard creation, so both runs report the same numbers. The CI examples in these docs pass it.

**Do not pass `--collect-atstart=no` or `--toggle-collect`.** hotpath relies on application threads collecting by default, and turns collection off only for its own background threads by flipping their flag once per session. With `--collect-atstart=no`, application threads never collect, so measured functions get no instructions, and the flip turns hotpath's background threads on instead, leaving a total made of profiler overhead. `--toggle-collect=<function>` flips the same per-thread flags on entry to and exit from matching functions, which breaks the counts in the same way.

## Timing and instruction counts in one report

A Callgrind run has no timing, and a native run has no instruction counts. **Merged** mode (native timing + Callgrind instruction counts in one report) runs the same build twice and combines the two: run it natively with a JSON report first, then under Callgrind with `HOTPATH_MERGE_REPORT` naming that report:

```bash
cargo build --release --example my_benchmark --features hotpath-alloc,hotpath-cloud,hotpath-callgrind

HOTPATH_OUTPUT_FORMAT=json HOTPATH_OUTPUT_PATH=native.json \
  target/release/examples/my_benchmark

HOTPATH_OUTPUT_FORMAT=json HOTPATH_OUTPUT_PATH=merged.json \
  HOTPATH_MERGE_REPORT=native.json \
  valgrind -q --tool=callgrind --instr-atstart=no \
  target/release/examples/my_benchmark
```

`merged.json` is the native report with the Callgrind run's instruction counts added, and `meta.run` set to `"merged"`:

- **Native sections**: timing, allocations, threads, channels, locks and every other section come from the native run. Both runs measure allocations; the native run's are kept.
- **Instruction counts** come from the Callgrind run, with its own call counts, which can differ slightly from the timing section's for a workload that is not deterministic.
- **Elapsed time** is the native run's on the timing and allocation sections, and never shown for the instruction counts, which were measured under Valgrind.

Merging needs the `hotpath-callgrind` and `hotpath-cloud` features (`hotpath` `0.30.0` or newer). The Callgrind run refuses a native report that does not belong to it: a different `hotpath` version, benchmark, `rustc`, OS, working directory, git commit or branch, policy file or CI run, a report that is not from a native run, or one without timing (a sampling rate of `0`). It also refuses when it did not count any instructions itself. A refused merge prints `merge failed: ...` with the reason and exits with status 1, whatever `HOTPATH_UPLOAD` says; it writes no report, uploads nothing and leaves an existing output file untouched. A program that panics skips the merge.

Both runs must do the same work: the same binary, arguments, environment, inputs and working directory, a fixed workload (fixed inputs and seeds, no network), and nothing the first run leaves behind (a database it fills, files it creates) changing the second.

In [hotpath-cloud](#cloud-ci), merged mode is the recommended way to benchmark in CI.

## What is counted

- **Inclusive**: a function's count includes instructions executed by functions it calls, like timing totals. Recursive invocations contribute to their enclosing calls, so the `Total` of a recursive function, which sums inclusive counts across all its invocations, counts the same instructions multiple times.
- **Async functions** count the instructions executed while their body is polled. Work in tasks spawned with `tokio::spawn` runs elsewhere and is not part of the caller's count.
- **Calls** come from hotpath's own counters, `Avg` is total divided by calls.
- `% Total` is relative to all instructions the program executed between guard creation and drop. hotpath's background threads (collectors and samplers) are excluded, so the total counts the instructions of application threads, including the instrumentation overhead executed on them.
- `measure_block!` blocks are not counted, nor are measured functions that a `macro_rules!` generates from one source position (a warning names them), nor sync `#[track_caller]` functions (counting them would break `Location::caller()`).

Counts are reproducible for deterministic workloads. Randomized hashing (`HashMap` with the default `RandomState`) and concurrent workloads can produce different counts between runs. Valgrind serializes thread execution, so counts from multi-threaded code may not represent native scheduling behavior.

## Comparing runs

Only compare counts produced by the same Valgrind version, Rust toolchain and target. The JSON report records the tool in the section's `tool` field (e.g. `callgrind-3.26.0`). Pin the rest of the runtime environment and dependencies too: libc versions, compiler flags and CPU-specific code paths can all change counts. Counts may differ between local and CI environments, even for the same commit.

## Cloud CI

Reproducible counts let [hotpath-cloud](cloud.md) judge changes in CPU work on shared GitHub runners, where timing is too noisy to judge small changes. A higher count means more work, which usually but not always means slower code. The recommended setup is [merged mode](#timing-and-instruction-counts-in-one-report): build the benchmark with `hotpath-alloc,hotpath-cloud,hotpath-callgrind`, run it natively, then under Callgrind in a pinned environment, which uploads one merged report. Its instruction counts and allocations are judged under `[functions.icount]` and `[functions.alloc]` of the [policy file](./regression_policy.md#functions-instructions), and its native timing is shown next to them. [CI integration](./ci_integration.md#profiling-modes) has the workflows; the [AI setup](./cloud.md#ai-setup-recommended) picks merged mode for suitable new benchmarks on its own.

## Build requirements

Attribution reads function symbols from Callgrind's output, so the binary must keep its symbol table: release builds work by default, but not with `strip = "symbols"` (or `strip = true`) in the profile.

Under the `hotpath-callgrind` feature, every `#[hotpath::measure]` body runs inside a small `#[inline(never)]` marker function that Callgrind attributes the instructions to. In native runs of the same build that costs one extra call per measured invocation.
