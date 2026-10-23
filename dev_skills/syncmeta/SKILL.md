---
name: syncmeta
description: Sync changes from the hotpath, hotpath-macros and hotpath-drain crates to their meta counterparts (hotpath-meta, hotpath-macros-meta and hotpath-drain-meta). Use when meta crates need to be updated with recent changes.
allowed-tools: Bash, Read, Edit, Write, Glob, Grep
---

# Sync Meta Crates

Sync recent changes from `hotpath` → `hotpath-meta`, `hotpath-macros` → `hotpath-macros-meta` and `hotpath-drain` → `hotpath-drain-meta`.

The meta crates are copies of the main crates used to profile the profiler itself. They must stay in sync with the source crates.

## Effective Approach

**DO NOT copy entire files from hotpath to hotpath-meta and then sed-replace.** This approach fails because the meta crates have many naming differences beyond simple `hotpath::` → `hotpath_meta::` substitution:

- Feature flags: `hotpath` → `hotpath-meta`, `hotpath-alloc` → `hotpath-alloc-meta`, `hotpath-cpu` → `hotpath-cpu-meta`, `hotpath-cloud` → `hotpath-cloud-meta`, `hotpath-prometheus` → `hotpath-prometheus-meta`, `hotpath-mcp` → `hotpath-mcp-meta`
- Crate imports: `hotpath_macros` → `hotpath_macros_meta`, `hotpath_drain` → `hotpath_drain_meta`
- Environment variables: `HOTPATH_FOCUS` → `HOTPATH_META_FOCUS`, `HOTPATH_EXCLUDE_WRAPPER` → `HOTPATH_META_EXCLUDE_WRAPPER`, `HOTPATH_OUTPUT_PATH` → `HOTPATH_META_OUTPUT_PATH`
- Self-instrumentation: lines like `#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]` exist in hotpath but must NOT exist in hotpath-meta
- `hotpath-drain-meta` is the simplest mirror: it has no features and no dependencies, so syncing `hotpath-drain` means applying the diff and dropping the `#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(...))]` lines. Nothing else is renamed - there is no `hotpath-drain-meta` feature flag.

**Instead, apply diffs to the existing meta files:**

1. Get the actual diff for each changed file: `git diff HEAD~N..HEAD -- crates/hotpath/src/path/to/file.rs`
2. Read the corresponding meta file
3. Apply the equivalent semantic changes, preserving all meta-specific naming

## Steps

1. **Check the last N commits** (default 1, user can specify more) for changes to `crates/hotpath/`, `crates/hotpath-macros/` and `crates/hotpath-drain/`:

```
git log --oneline -N
git diff HEAD~N..HEAD --name-only -- crates/hotpath/src/ crates/hotpath-macros/src/ crates/hotpath-drain/src/
```

2. **For each changed file**, get the source diff:

```
git diff HEAD~N..HEAD -- crates/hotpath/src/path/to/file.rs
git diff HEAD~N..HEAD -- crates/hotpath-drain/src/lib.rs
```

3. **Read the corresponding meta file** and apply the equivalent changes using Edit tool. The meta files are at:
   - `crates/hotpath/src/**` → `crates/hotpath-meta/src/**`
   - `crates/hotpath-macros/src/**` → `crates/hotpath-macros-meta/src/**`
   - `crates/hotpath-drain/src/**` → `crates/hotpath-drain-meta/src/**`

4. **Verify** the meta crates compile:

```
cargo check -p hotpath-drain-meta
cargo check -p hotpath-meta
cargo check -p hotpath-meta --features hotpath-meta
cargo check -p hotpath-macros-meta --features hotpath-meta
cargo check -p hotpath-meta --features hotpath-meta,hotpath-alloc-meta
cargo check -p hotpath-meta --features hotpath-meta,hotpath-alloc-meta,hotpath-prometheus-meta
```

The no-feature check covers the `lib_off` path. Every `*-meta` sub-feature (`hotpath-alloc-meta`, `hotpath-cpu-meta`, `hotpath-cloud-meta`, `hotpath-prometheus-meta`, `hotpath-mcp-meta`) requires `hotpath-meta` and hits a `compile_error!` on its own, so always combine them with `hotpath-meta`. Add `hotpath-cloud-meta` / `hotpath-mcp-meta` / `hotpath-cpu-meta` to the last command when the synced files touch those subsystems.

5. Report a summary of what was synced.

## Rules

- Only sync `src/` files, never `Cargo.toml` or other config files. `examples/` and `tests/` (including the miri test in `hotpath-drain`) are not mirrored either.
- If no changes were made to the source crates in the specified commits, report that and exit.
- NEVER copy entire files and do bulk find-and-replace. Always apply diffs to existing meta files.
- Preserve ALL meta-specific naming conventions (feature flags, env vars, crate names, self-instrumentation removal).
- Lines with `#[cfg_attr(feature = "hotpath-meta", hotpath_meta::...)]` in hotpath are self-instrumentation and must NOT be copied to meta crates.
- The meta feature flags are `hotpath-meta`, `hotpath-alloc-meta`, `hotpath-cpu-meta`, `hotpath-cloud-meta`, `hotpath-prometheus-meta`, `hotpath-mcp-meta` (NOT `hotpath`, `hotpath-alloc`, ...). There is no `hotpath-off-meta`; the disabled path is the no-feature build.
