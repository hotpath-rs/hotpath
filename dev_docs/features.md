# Features Reference

Where things are defined, plus gotchas the code site can't show. The code is the source of truth for behavior.

## Sources of truth

- **Feature flags**: `[features]` in `crates/hotpath/Cargo.toml`. User-facing descriptions are in the per-subsystem pages under `docs/src/` (`SUMMARY.md` is the index).
- **Attribute macros** and their parameters: doc comments in `crates/hotpath-macros/src/lib.rs`.
- **Declarative macros**: `crates/hotpath/src/lib_on.rs`; wrapper macros (`channel!`, `stream!`, `io!`, ...) are `#[macro_export]` in the matching `lib_on/<subsystem>.rs`. Every macro has a no-op twin in `lib_off.rs`.
- **Builder API**: `HotpathGuardBuilder` in `lib_on/hotpath_guard.rs`.
- **Environment variables**: user-facing reference in `docs/src/configuration.md`; find parse sites with `rg 'HOTPATH_' crates/hotpath/src crates/hotpath/bin`.
- **Report JSON types**: `crates/hotpath/src/json.rs` and `src/json/formatted.rs`; report builders in `lib_on/report.rs`.

## Features

- `tokio` is only for tokio-specific integrations; async function profiling is runtime-agnostic.
- `sqlx` / `toasty` are generic `tracing` layers with no sqlx/toasty dependency; filtering out the `sqlx::query` / `toasty::query` targets silently disables them.
- `json` gates `hotpath::json` without pulling the profiler in (used by hotpath-backend to parse reports).
- `demo` exists only for the TUI demo traffic (`bin/hotpath/cmd/console/demo.rs`).

## Cloud

- `hotpath-cloud` upload: `lib_on/cloud.rs`, called from `HotpathGuard::drop`. Report provenance: `lib_on/report_meta.rs`, `lib_on/git_info.rs`, `lib_on/ci_info.rs`. Policy file lookup: `json/policy_file.rs`.
- User docs: the "hotpath Cloud" pages in `docs/src/SUMMARY.md`.
- `cloud` CLI: `bin/hotpath/cmd/cloud.rs`, one file per command under `cmd/cloud/`, shared HTTP plumbing in `cmd/cloud/api.rs`.
- `json/cloud_api.rs` is the client contract hotpath-backend imports; bodies the CLI only prints are owned by the backend. `validate_benchmark_name` must stay identical to the backend's copy.
- The cloud path (upload or JSON output with the feature on) ignores display limits and renders exact precision; the live metrics server never does.
- Tests: `tests/cloud_*.rs` (against a `mockito` server).

## Macros

- Label uniqueness: `__unique_label!` in `hotpath-macros/src/lib_on.rs`, tests in `tests/unique_labels.rs` and `tests/cdylib.rs`. Duplicates surface only at link time, not in `cargo check` or clippy. Symbols must stay plain C identifiers or cdylib links break.
- `#[measure]` on a bare impl method needs `impl_type = "Type"` for `hotpath-cpu` attribution (see `architecture.md`).
- `channel!` on bounded std `sync_channel` / `futures_channel::mpsc` needs `capacity = N`, otherwise it panics at runtime.
- `channel!` / `stream!` / `io!`: `iter = true` grows profiler state with instance churn.
- A new HTTP/SQL front-end must be added to the `cfg_if!` gate in `lib_on/caller_stack.rs` and `POLL_WRAPPER_ALWAYS` in `lib_on/functions.rs`, or the `Source` column stays empty.
- `axum!` (`lib_on/server.rs`, layer in `lib_on/server/axum_08.rs`) must wrap the finished router.
- Report `location` fields: `lib_on/locations.rs`.

## Env vars

- `HOTPATH_TIME_EXCLUSIVE` (exclusive function time): `lib_on/functions/exclusive.rs`; async guards must never open a frame there, their polls do (`futures/wrapper.rs`).
- `HOTPATH_KEEP_INLINE` is read at macro expansion time; touch the source or `cargo clean` after toggling it.
- `HOTPATH_USER_METADATA` is parsed in `HotpathGuardBuilder::build` (the builder can add pairs), not a `LazyLock`; it only reaches the final report.
