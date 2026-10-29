# Architecture Reference

Code map for runtime internals. See `AGENTS.md` for the high-level pipeline.

## Background workers

Each subsystem spawns a worker thread named `hp-<subsystem>` from its `lib_on/<subsystem>.rs` (`rg '"hp-'` for the full list). Exception: `hp-functions` lives in `lib_on/hotpath_guard.rs`. `hp-threads` and `hp-runtime` are samplers, not queue consumers.

## Servers

- Metrics HTTP server: `src/metrics_server.rs`, routes in the `Route` enum in `src/json.rs`. Auth token comparison is shared with the MCP server in `src/auth.rs`.
- Prometheus exporter: `src/prometheus_server.rs` (`hotpath-prometheus` feature), histogram conversion in `lib_on/native_histograms.rs`. A query timeout must return `503`, never an empty body (empty scrapes look like counter resets).
- MCP server: `src/mcp_server.rs` (`hotpath-mcp` feature); the tools are its `#[tool]` methods.

## Tokio runtime monitoring

`lib_on/tokio_runtime.rs`. Some metrics need `RUSTFLAGS="--cfg tokio_unstable"`.

## Instruction counts (`hotpath-callgrind`, x86_64 Linux)

- `lib_on/functions/callgrind.rs` - marker registry, session start/finish, report section.
- `lib_on/valgrind.rs` - generic Valgrind client request (`asm!`) and `ON_VALGRIND`; `lib_on/functions/callgrind/request.rs` - Callgrind request codes.
- `lib_on/functions/callgrind/parser.rs` - Callgrind dump parsing, fixture in `callgrind/fixtures/`.
- Markers are emitted in `measure_impl` (`hotpath-macros/src/lib_on.rs`).
- `lib_on/merge.rs` - `HOTPATH_MERGE_REPORT`: a Callgrind run merges a native run's JSON report into its own (wired in `HotpathGuard::drop`); tests in `tests/functions_icount.rs`.

Gotchas:
- A client-request dump is `<out-file>.<n>`, not the bare out file (that one is the exit dump).
- Markers need a per-site constant in their body, or LLVM merges identical ones into one symbol.

## CPU sampling (`hotpath-cpu`, macOS and Linux)

- `bin/hotpath-samply/main.rs` - wrapper child that runs `samply record --pid <host>`.
- `lib_on/functions/cpu/autospawn.rs` - wrapper lifecycle.
- `lib_on/functions/cpu/samply.rs` - profile parsing and symbol attribution.

Gotchas:
- The profile is samply's Firefox Profiler JSON (gzipped), not pprof.
- Only inherent impl and free function symbols match; trait impl methods (`<Type as Trait>::method`) never do. Hence `impl_type` on bare `#[measure]` impl methods.
