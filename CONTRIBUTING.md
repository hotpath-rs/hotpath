## Submitting Changes

1. Create a feature branch from `main`
2. Make your changes
3. Ensure all the [CI checks](#ci-checks) pass
4. Open a pull request against `main`
5. Always check `Allow edits and access to secrets by maintainers` so we can push fixes or rebases directly to your branch ([docs](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/working-with-forks/allowing-changes-to-a-pull-request-branch-created-from-a-fork))

## `meta` crates explained

Project maintains a complete copy of `hotpath` (`hotpath-meta`), `hotpath-macros` (`hotpath-macros-meta`) and `hotpath-drain` (`hotpath-drain-meta`). All changes must be mirrored in their corresponding `-meta` crates. This adds some maintenance overhead, but it allows to benchmark the library using itself, which is an invaluable source of performance data and optimization insights.

A full copy is needed because a crate cannot depend on itself. Extracting shared core is also impractical, because `hotpath` uses a custom instrumentation logic (like `#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]` calls). If you have ideas for a cleaner way to implement self-profiling without full crate duplication, I'm open to suggestions.

To mirror changes from the source crates into the meta crates, you can use the [`syncmeta`](dev_skills/syncmeta/SKILL.md) LLM skill. It applies diffs from `hotpath`/`hotpath-macros`/`hotpath-drain` to their meta counterparts while preserving meta-specific naming (feature flags, env vars, crate imports).

## Benchmarking `hotpath` 

### Self benchmarks

Install [just](https://github.com/casey/just) and run:

```bash
just bench_meta
```

Starts a hotpath TUI for 5 seconds, gathers performance metrics and prints the report on exit. 

- `HOTPATH_TUI_TAB` - set values from 1 to 6, to open a different TUI tab and execute different codepaths in the benchmark (default `1`)
- `HOTPATH_BENCH_RELEASE` - set to `true` to run benchmarks with `--release` profile (default `false`)
- `HOTPATH_TUI_REFRESH_INTERVAL_MS` - configure data refresh interval, lower values will produce more data (default `10`)
- `HOTPATH_TUI_AUTO_EXPAND_LOGS` - Auto-open the logs panel once initial data arrives and pin selection to the given table index. Set to an integer (e.g. `0` for the first row, `2` for the third) (default: unset). 
- `HOTPATH_META_FOCUS` - filter which methods appear in the benchmark report by name. Plain text does substring matching; wrap in `/pattern/` for regex (e.g. `HOTPATH_META_FOCUS="/^(compute|process)/"`).

### Overhead benchmarks

Each `benchmark_*` example hammers a single instrumented codepath in a tight, uncontended loop and prints total time plus per-operation overhead on exit. Run with `--features hotpath` to measure instrumentation cost; omit it for the uninstrumented baseline. The iteration count defaults to 1,000,000 and is configurable via `HOTPATH_BENCH_RUNS`.

#### Timing

```bash
cargo run --example benchmark_noop --features hotpath --release
```

#### Allocations

```bash
cargo run --example benchmark_alloc --features='hotpath-alloc' --release
```

#### Instant

`hotpath` measures time with a custom `Instant` (`mach_absolute_time` on macOS, `quanta` on Linux) instead of `std::time::Instant`. This benchmark compares the `now()` call overhead of both clocks:

```bash
cargo run -p test-tokio-async --example benchmark_instant --features hotpath --release
```

#### Mutexes and RwLocks

Each benchmark runs both an uninstrumented **baseline** (raw lock) and the instrumented `hotpath::mutex!` / `hotpath::rw_lock!` version in a single command, printing their per-op cost side by side so the delta isolates the instrumentation overhead. RwLock benchmarks run a write loop followed by a read loop, reporting each separately.

```bash
cargo run -p test-mutex-std --example benchmark_mutex_std --features hotpath --release
cargo run -p test-mutex-tokio --example benchmark_mutex_tokio --features hotpath --release
cargo run -p test-mutex-parking-lot --example benchmark_mutex_parking_lot --features hotpath --release
cargo run -p test-mutex-async-lock --example benchmark_mutex_async_lock --features hotpath --release
cargo run -p test-rw-lock-std --example benchmark_rw_lock_std --features hotpath --release
cargo run -p test-rw-lock-tokio --example benchmark_rw_lock_tokio --features hotpath --release
cargo run -p test-rw-lock-parking-lot --example benchmark_rw_lock_parking_lot --features hotpath --release
cargo run -p test-rw-lock-async-lock --example benchmark_rw_lock_async_lock --features hotpath --release
```

#### Channels

Each benchmark runs an uninstrumented **baseline** (raw channel) and the **instrumented** channel in a single command and prints their per-op cost side by side. The delta vs baseline isolates the instrumentation overhead.

```bash
cargo run -p test-channels-std --example benchmark_channel_std --features hotpath --release
cargo run -p test-channels-crossbeam --example benchmark_channel_crossbeam --features hotpath --release
cargo run -p test-channels-tokio --example benchmark_channel_tokio --features hotpath --release
cargo run -p test-channels-flume --example benchmark_channel_flume --features hotpath --release
cargo run -p test-channels-asc --example benchmark_channel_asc --features hotpath --release
cargo run -p test-channels-ftc --example benchmark_channel_ftc --features hotpath --release
```

#### SQL

Each benchmark runs both an uninstrumented **baseline** and the instrumented version in a single command, hammering the same point lookup against an in-memory SQLite database and printing their per-op cost side by side, so the delta isolates the per-query instrumentation overhead (tracing dispatch / instrumentation callback, normalization keying, and event enqueue). The sqlx and Toasty baselines run before the hotpath `tracing` layer is installed; the Diesel baseline uses a connection established before `instrument_diesel_sql()` is called. Iteration count defaults to 50,000 (`HOTPATH_BENCH_RUNS`).

```bash
cargo run -p test-sqlx-08 --example benchmark_sql_sqlx --features hotpath --release
cargo run -p test-diesel --example benchmark_sql_diesel --features hotpath --release
cargo run --manifest-path crates/test-toasty/Cargo.toml --example benchmark_sql_toasty --features hotpath --release
```

Each library also has a PostgreSQL variant that runs the same point lookup against a real server, so every op includes a TCP round trip - closer to production numbers, but the round trip dominates and deltas below ~1µs are within run-to-run noise. Iteration count defaults to 10,000. Start the database first with `docker compose up -d postgres`:

```bash
cargo run -p test-sqlx-08 --example benchmark_sql_sqlx_postgres --features hotpath --release
cargo run -p test-diesel --example benchmark_sql_diesel_postgres --features hotpath,pg --release
cargo run --manifest-path crates/test-toasty/Cargo.toml --example benchmark_sql_toasty_postgres --features hotpath --release
```

#### HTTP

Runs both an uninstrumented **baseline** (raw reqwest client) and the `hotpath::http!` wrapped version in a single command, hammering a local `tiny_http` server over a kept-alive loopback connection, so the delta isolates the per-request overhead of the middleware hop, endpoint normalization, and event enqueue. Note that the full loopback round trip (~40 µs/request) dominates each op, so deltas below ~1 µs are within run-to-run noise. Iteration count defaults to 10,000 (`HOTPATH_BENCH_RUNS`).

```bash
cargo run -p test-reqwest-013 --example benchmark_http_reqwest --features hotpath --release
```

#### Byte-level I/O

Each benchmark runs both an uninstrumented **baseline** (raw `Read`/`Write` value) and the `hotpath::io!` wrapped version in a single command, printing their per-op cost side by side so the delta isolates the per-operation instrumentation overhead. Iteration counts are fixed per example (200,000 file ops, 10,000 network round trips after a 1,000-op warmup).

The file benchmark hammers 64-byte reads and writes against temp files; the TCP benchmarks echo 64-byte chunks against an in-process loopback server (sync `TcpStream` and tokio `AsyncRead`/`AsyncWrite` variants), so no external services are required:

```bash
cargo run -p test-io --example benchmark_file_io --features hotpath --release
cargo run -p test-io --example benchmark_tcp_io --features hotpath --release
cargo run -p test-io --example benchmark_tokio_tcp_io --features hotpath --release
```

The Redis variant sends raw RESP PING/SET/GET round trips over a plain vs instrumented `TcpStream` to a real server, so every op includes a TCP round trip. Start the database first with `docker compose up -d redis` (host port 6390); the benchmark skips when nothing listens there:

```bash
cargo run -p test-io --example benchmark_redis_io --features hotpath --release
```

#### Futures and Streams

```bash
cargo run -p test-futures --example benchmark_future --features hotpath --release
cargo run -p test-streams --example benchmark_stream --features hotpath --release
```

#### Event transport

Every subsystem (functions, channels, streams, futures, rw_locks, mutexes, sql) records events into per-thread lock-free chunked SPSC queues (the `hotpath-drain` crate, `crates/hotpath-drain/src/lib.rs`): the hot path is a plain slot store plus one `Release` publish, with no mutex or RMW atomic. Each subsystem's background worker is the single consumer and sweeps all queues every 50ms and once more at shutdown, so anything recorded on any thread - even one still parked at exit - appears in the report.

The `drain` and `drain-criterion` hotpath-cloud benchmarks run one lockstep scenario (`crates/hotpath-drain/examples/support/lockstep.rs`) with `hotpath-meta` profiling the queue: once through the `throughput` example, five times through the Criterion benchmark in `crates/test-drain-criterion`:

```bash
cargo run -p hotpath-drain --example throughput --release --features hotpath-alloc-meta
HOTPATH_META_CRITERION_ITERATIONS=5 cargo bench -p test-drain-criterion --bench drain --features hotpath-alloc-meta -- --test --exact drain/lockstep
```

### Samply traces 

Analyze [Samply](https://github.com/mstange/samply) traces by running the instrumented benchmarks:

```bash
cargo install --locked samply
```

#### Timing

```bash
cargo build --example benchmark_noop --features hotpath --profile profiling && HOTPATH_BENCH_RUNS=5000000 samply record './target/profiling/examples/benchmark_noop'
```

#### Allocations

```bash
cargo build --example benchmark_alloc --features='hotpath-alloc' --profile profiling && samply record './target/profiling/examples/benchmark_alloc'
```

## Building the documentation

The mdBook source lives in `docs/`. Install the dependencies:

- https://github.com/rust-lang/mdBook
- https://github.com/pawurb/mdbook-reading-time 
- https://github.com/pawurb/mdbook-assets-hash 
- `mdbook-blank-links` (the `blank-links` preprocessor in `docs/book.toml`, not published on crates.io)

```bash
cargo install mdbook
cargo install mdbook-reading-time
cargo install mdbook-assets-hash
```

`just docs` - start the mdBook dev server with live reload on `http://localhost:3000`
(opens your browser automatically; rebuilds on file changes).

The Axum web server that builds and serves the production `hotpath.rs` site lives in a
separate private repository and is not part of this repo.

## CI checks

CI runs on `ubuntu-latest` against Rust `1.89`, `stable`, and `nightly`. You can run the same checks locally:

### Compilation checks

```bash
cargo check
cargo check --all-features
cargo check --features hotpath
cargo check --features "hotpath-alloc"
cargo check --features "hotpath-mcp"
cargo check --features "hotpath-prometheus"
cargo check --features "hotpath,hotpath-alloc-meta"
cargo check --features "hotpath-alloc,hotpath-alloc-meta"
cargo check -p hotpath --bin hotpath --features=tui
cargo check --features='tui,hotpath-alloc-meta,hotpath-mcp,hotpath-mcp-meta,dev' --bin hotpath
cargo check -p hotpath --bin hotpath --features=cloud
```

### Formatting and linting

```bash
cargo fmt --all --check
cargo clippy --all --features hotpath -- -D warnings
cargo clippy --all --all-features -- -D warnings
cargo clippy --all --features "hotpath-alloc" -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc -p hotpath --no-deps --features hotpath-alloc
```

### Miri

Checks the unsafe code in `hotpath-drain` (requires the nightly `miri` component: `rustup +nightly component add miri`):

```bash
cargo +nightly miri test -p hotpath-drain
```

### Tests

```bash
cargo test --lib --features hotpath
cargo test -p hotpath --bin hotpath --features=tui
cargo test -p hotpath --features cloud --test cloud_cli
cargo run -p test-all-features --example all_noop
```

Integration tests live in `crates/hotpath/tests/` and run one file at a time, for example:

```bash
cargo test --features hotpath --test functions_timing -- --nocapture --test-threads=1
cargo test --features hotpath-prometheus --test prometheus_metrics -- --nocapture --test-threads=1
```

Or run all integration tests at once with the `test_all` recipe in the `justfile`:

```bash
just test_all
```

## Crates

| Crate | Description |
|-------|-------------|
| `hotpath` | Core library - profiling runtime, reporting, metrics server, MCP server, TUI binary |
| `hotpath-meta` | Mirror of the `hotpath` library, used to profile the profiler itself. |
| `hotpath-macros` | Procedural macros (`#[measure]`, `#[main]`, `#[future_fn]`, etc.) |
| `hotpath-macros-meta` | Mirror of the `hotpath-macros` library, used to profile the profiler itself. |
| `hotpath-drain` | Lock-free per-thread event queues with a single drain - the transport layer between instrumented code and the background workers |
| `hotpath-drain-meta` | Mirror of the `hotpath-drain` library, used to profile the profiler itself. |
| `test-*` | One integration-test and example crate per instrumented subsystem or third-party integration. The current list is the `members` array in the root `Cargo.toml`. |
| `test-toasty` | Toasty integration tests. Not a workspace member (conflicting `links = "sqlite3"`), build it with `--manifest-path crates/test-toasty/Cargo.toml`. |

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
