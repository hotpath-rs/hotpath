# Deterministic instruction counts with Callgrind

Wall-clock timings in CI vary by 5-30% between runs: frequency scaling, noisy neighbours and scheduling all leak into the numbers, so small regressions drown in noise. `hotpath` can instead report how many instructions each measured function executed, counted by [Valgrind's Callgrind](https://valgrind.org/docs/manual/cl-manual.html). Callgrind simulates the CPU in software, so the count is the same on every run and on every machine with the same toolchain and Valgrind version, including standard GitHub-hosted runners that expose no hardware performance counters.

Instruction counts are a proxy for time, not a replacement. Cache misses, branch mispredictions and stalls cost time without adding instructions, and waiting (sleeps, I/O, locks, channels) executes none. Use them next to timing reports, as a stable signal for CPU work.

Supported on x86_64 Linux only. On other targets the feature compiles, but a Valgrind run is not detected and the timing report is printed.

## Running under Callgrind

Install Valgrind (`sudo apt-get install -y valgrind` on Ubuntu), then enable the `hotpath-callgrind` feature:

`Cargo.toml`
```toml
[features]
hotpath = ["hotpath/hotpath"]
hotpath-callgrind = ["hotpath/hotpath-callgrind"]
```

and run the program under Callgrind, for example through Cargo's target runner:

```bash
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" \
  cargo run --release --features='hotpath,hotpath-callgrind'
```

Under Valgrind the timing report is replaced by an instructions report (durations measured inside the simulator are meaningless):

```
instructions - Inclusive instructions executed per function (Callgrind Ir).
+------------------------------+-------+---------+----------+---------+
| Function                     | Calls | Avg     | Total    | % Total |
+------------------------------+-------+---------+----------+---------+
| instructions::recursive      | 4     | 4500564 | 18002258 | 45.31%  |
+------------------------------+-------+---------+----------+---------+
| instructions::async_fn       | 2     | 7200797 | 14401595 | 36.25%  |
+------------------------------+-------+---------+----------+---------+
| instructions::free_fn        | 2     | 1800012 | 3600024  | 9.06%   |
+------------------------------+-------+---------+----------+---------+
```

The same build run natively prints the usual timing report, so one binary serves both runs. An explicit `HOTPATH_REPORT=functions-timing` is still honoured under Valgrind, but its durations are meaningless. Expect the Callgrind run to be 50-100x slower than a native one, and its threads to run one at a time.

Callgrind writes its counts to `callgrind.out.<pid>` files in the working directory. If you pass a custom `--callgrind-out-file`, set `HOTPATH_CALLGRIND_OUT_FILE` to the same value (`%p` and `%q{VAR}` are expanded like Valgrind does).

## What is counted

- **Inclusive**: a function's total includes everything it calls, like timing totals. A recursive function counts nested calls once per enclosing level, so its total is the sum of every call's inclusive count.
- **Async functions** count the instructions executed while their body is polled. Work in tasks spawned with `tokio::spawn` runs elsewhere and is not part of the caller's count.
- **Calls** come from hotpath's own counters, `Avg` is total divided by calls.
- `% Total` is relative to all instructions the program executed between guard creation and drop.
- `measure_block!` blocks are not counted, nor are measured functions that a `macro_rules!` generates from one source position (a warning names them), nor sync `#[track_caller]` functions (counting them would break `Location::caller()`).

Counts are exact for deterministic code. Randomized hashing (`HashMap` with the default `RandomState`) and contended multi-threaded code vary slightly between runs.

## Comparing runs

Only compare counts produced by the same Valgrind version, Rust toolchain and target. The JSON report records the tool in the section's `tool` field (e.g. `callgrind-3.24.0`). Counts from a local Docker run and from CI usually differ, even for the same commit.

## Build requirements

Attribution reads function symbols from Callgrind's output, so the binary must keep its symbol table: release builds work by default, but not with `strip = "symbols"` (or `strip = true`) in the profile.

Under the `hotpath-callgrind` feature, every `#[hotpath::measure]` body runs inside a small `#[inline(never)]` marker function that Callgrind attributes the instructions to. In native runs of the same build that costs one extra call per measured invocation.
