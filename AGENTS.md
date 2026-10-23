# AGENTS.md

This file provides guidance to AI agent when working with code in this repository.

## Project Overview

hotpath-rs is a lightweight, feature-gated Rust profiler that tracks function execution time, memory allocations, channels, streams, futures, locks, SQL queries, HTTP requests, byte-level I/O, and threads. All instrumentation is behind the `hotpath` Cargo feature: with it off, every macro is a no-op and no dependencies are compiled.

Workspace layout:
- `crates/hotpath` - Main library with profiling runtime, reporting, metrics/MCP servers, and the TUI/CLI binaries
- `crates/hotpath-macros` - Procedural macros (`#[measure]`, `#[main]`, `#[future_fn]`, ...)
- `crates/hotpath-drain` - Lock-free per-thread SPSC event queues and their registry; re-exported inside hotpath as `crate::batch`
- `crates/test-*` - One integration-test crate per instrumented subsystem or third-party integration; the current list is the `members` array in the root `Cargo.toml`
  - `test-toasty` is NOT a workspace member: toasty's rusqlite and the workspace's sqlx-sqlite have conflicting `links = "sqlite3"` values, so it's built via `cargo run --manifest-path crates/test-toasty/Cargo.toml ...`
- `crates/hotpath-meta` / `crates/hotpath-macros-meta` / `crates/hotpath-drain-meta` - Copies of hotpath used to profile the profiler itself (not intended for external use)
- `docs/` - mdBook source for the hotpath.rs documentation site (the Axum web server that builds/serves it lives in a separate private repo at `../hotpath-backend`)
  - `docs/agent/` - markdown-only agent procedures (not in `SUMMARY.md`), served raw by the backend at `https://hotpath.rs/agent/<name>.md` with `{{HOTPATH_VERSION}}`-style placeholders filled in
- `skills/` - public agent skills installed with `npx skills add hotpath-rs/hotpath` (only `hotpath-rs`); repo-internal skills live in `dev_skills/`

## Reference Docs

The dev_docs point to where things are defined in source and record gotchas the code can't show - read them only when a task needs the specifics:

- `dev_docs/features.md` - Where features, macros, the builder API, env vars and cloud code are defined, plus gotchas
- `dev_docs/architecture.md` - Background workers, servers, CPU sampling code map
- `dev_docs/tui.md` - TUI build/usage and code map
- `dev_docs/testing.md` - Integration-test patterns. Read before writing or modifying an integration test.
- `CONTRIBUTING.md` - Meta-crate mirroring, benchmark commands, samply tracing, local CI check commands, docs build prerequisites

Keep dev_docs and this file pointer-style: where something lives plus at most a one-line gotcha. Never describe exact logic, wire formats or design rationale there; that belongs in code comments.

## Development Commands

```bash
cargo build                                # build
cargo build --features hotpath             # build with profiling enabled
cargo check --bin hotpath --features tui   # check the TUI binary compiles

# Run an example from a test crate (each example lists its exact run command in its top comment)
cargo run -p test-tokio-async --example basic --features hotpath
cargo run -p test-channels-tokio --example basic_tokio --features hotpath

# Profiling modes are combined via features, e.g.
cargo run -p test-tokio-async --features='hotpath-alloc' --example basic
```

Just recipes:
```bash
just test_all      # Run all integration tests
just docs          # Serve the mdbook docs locally with live reload (http://localhost:3000)
```

TUI quickstart (details in `dev_docs/tui.md`): run a profiled example in one terminal (metrics server starts on port 6770 by default), then `cargo run --bin hotpath --features tui -- console --metrics-port 6770` in another.

## Architecture

**Profiling pipeline**: instrumented code -> per-thread lock-free SPSC queue (`crates/hotpath-drain`) -> per-subsystem `hp-<subsystem>` worker thread (sweeps every 50ms and once more at shutdown) -> statistics -> report on guard drop. The producer path must stay free of locks and RMW atomics.

**Feature gating**: `lib.rs` orchestrates via `#[cfg(feature = ...)]`; `lib_on.rs` is the enabled implementation, `lib_off.rs` the no-op stubs. Every public macro must exist in both. Time profiling uses `MeasurementGuard` in `lib_on/functions/timing/guard.rs`; allocation profiling uses a custom global allocator with `MeasurementGuardSync` / `MeasurementGuardAsync` in `lib_on/functions/alloc/guard.rs`.

**Async caveats**: async function profiling works on any async runtime with no runtime-specific feature flag (see `crates/test-smol-async`); the `tokio` feature is only needed for tokio-specific integrations (tokio channels/locks, async `io!` traits, `tokio_runtime!()`). Async allocation profiling is measured per `poll()` (`lib_on/futures/wrapper.rs`).

**Servers**: localhost metrics HTTP server (port 6770, on by default, feeds the TUI) in `metrics_server.rs` with routes in the `Route` enum in `src/json.rs`; optional MCP server (`hotpath-mcp`, port 6771) in `mcp_server.rs`.

**CPU sampling** (`hotpath-cpu`): external `samply` worker; see `dev_docs/architecture.md`.

**Source tracking**: `lib_on/caller_stack.rs` gives SQL/HTTP entries their `source` (innermost instrumented caller).

### Key Files

- `crates/hotpath/src/lib.rs` / `lib_on.rs` / `lib_off.rs` - Entry points (feature orchestration, enabled impl, no-op stubs)
- `crates/hotpath-macros/src/lib.rs` - Procedural macro implementations
- `crates/hotpath/src/lib_on/<subsystem>.rs` (+ same-named subdir) - One module per instrumented subsystem: `functions` (timing/alloc, + `functions/cpu/` for sampling), `channels`, `streams`, `futures`, `rw_locks`, `mutexes`, `sql`, `http`, `server`, `io`, `threads`, `tokio_runtime`, `debug`
- `crates/hotpath/bin/hotpath/` - TUI binary (`cmd/console/` holds app state, views, HTTP client)
- `crates/hotpath/bin/hotpath-samply/` - CPU sampling wrapper binary

### Documentation

The mdBook source lives in `docs/` (book.toml, src/, theme/). Serve a live-reloading local preview with `just docs` (`mdbook serve` on `http://localhost:3000`). Requires `mdbook`, `mdbook-assets-hash`, `mdbook-reading-time`, and `mdbook-blank-links` on PATH. The Axum web server that builds and serves the production site lives in a separate **private** repo at `../hotpath-backend`, which consumes `docs/` via a `html_src` symlink.

## Code style

Never use `super` for imports, only `crate` and absolute paths.

Default to `pub(crate)` for new functions, structs, and fields. Only use `pub` when the item is part of the public API or re-exported from `lib.rs`.

Never use em dashes. Always use a regular hyphen (-) instead. This applies everywhere, especially in code comments.

NEVER use `mod.rs` files, so instead of `functions/mod.rs` use `functions.rs`.

Read environment variables through a `static NAME: LazyLock<T>` evaluated once (see `ENTRIES_LIMIT` in `lib_on/hotpath_guard.rs`), not through a function that re-reads the env on every call. Exceptions are values the builder can override at guard creation, which are parsed in `HotpathGuardBuilder::build`.

Every example in a test crate contains its exact cargo command in the top comment, usually as a `//! Run with:` header (a descriptive module doc may precede it).

## Other

Never read hotpath-meta, hotpath-macros-meta and hotpath-drain-meta crates when exploring and planning. Changes to `crates/hotpath` / `crates/hotpath-macros` / `crates/hotpath-drain` must eventually be mirrored into their `-meta` counterparts (see CONTRIBUTING.md; the `dev_skills/syncmeta` skill applies the diffs), but perform that mirroring only when explicitly asked - it is triggered as a separate step, not as part of every change.

NEVER instrument hotpath-meta crates using hotpath_meta, it won't work.

NEVER run "just test_all" unless explicitly asked, it's slow.
