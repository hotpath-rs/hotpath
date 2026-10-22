---
name: hotpath_init
description: Configure hotpath profiling in a Rust project. Adds the hotpath dependency with feature-gated setup, instruments main with hotpath::main, functions with measure/measure_all, and wraps channels, mutexes, rw_locks, streams, futures, reqwest clients, axum routers and byte-level I/O with hotpath macros. Use when the user wants to add or set up hotpath profiling in a crate.
allowed-tools: Bash, Read, Edit, Write, Glob, Grep
---

# Initialize hotpath Profiling

Set up [hotpath](https://hotpath.rs) profiling in the current Rust project. The setup is fully feature-gated: zero compile-time and runtime overhead unless the `hotpath` feature is explicitly enabled. All macros are noops when the feature is off, so no `cfg_attr` wrapping is needed.

## Documentation

The hotpath docs are the reference for every macro, cargo feature, parameter and caveat; this skill only holds the procedure. Every page of https://hotpath.rs is served as markdown when requested with an `Accept: text/markdown` header:

```bash
curl -sL -H 'Accept: text/markdown' https://hotpath.rs/functions
```

Before each step, fetch the pages it names and follow them instead of relying on memory. Keep `-L` (some pages redirect) and the header (there are no `.md` URLs). A `#fragment` is not sent to the server: fetch the whole page and find the heading. Fetch a page only when the project uses what it covers.

The site documents the latest hotpath release, which can be newer than the version the project uses. Never change the project's `hotpath` version because the docs show a newer one. If the docs describe something that version does not have, tell the user it needs a newer hotpath instead of working around it.

| Page | Covers |
|---|---|
| https://hotpath.rs/ | basic setup, example report |
| https://hotpath.rs/profiling_modes | `#[hotpath::main]` vs `HotpathGuardBuilder`, report options, TUI, optional vs non-optional dependency |
| https://hotpath.rs/functions | `main`, `measure`, `measure_all`, `skip`, `measure_block!`, allocation profiling, custom allocators |
| https://hotpath.rs/data_flow | `channel!`, `stream!`, `future!`: supported libraries and their cargo features, `hotpath::wrap::` types, call-site aggregation, `capacity` |
| https://hotpath.rs/locks | `mutex!`, `rw_lock!`: supported libraries and their cargo features, wrapped types |
| https://hotpath.rs/io_tracing | `io!`, `io_unwrap`, measured layers, compression encoders |
| https://hotpath.rs/sql_tracing | sqlx, Diesel and Toasty query profiling |
| https://hotpath.rs/http_tracing | reqwest and ureq client profiling |
| https://hotpath.rs/axum_tracing | axum router profiling, route scoping of SQL and HTTP |
| https://hotpath.rs/tokio_runtime | `tokio_runtime!()` metrics |
| https://hotpath.rs/configuration | every environment variable |
| https://hotpath.rs/regression_policy | the policy file of step 7 |

## Steps

### 1. Inspect the project

- Find the binary crate(s) and the `main` function. If there is no `main` you control (e.g. a library or a test harness), use the `HotpathGuardBuilder` API instead of `#[hotpath::main]` (see step 3).
- Detect the async runtime (`tokio`, `smol`, none) and which instrumentable primitives the code uses: channels, `Mutex` and `RwLock`, futures streams, SQL (sqlx, diesel, toasty), HTTP clients (reqwest, ureq; note the major version), axum routers (find where the `Router` is finished and passed to `axum::serve`), byte-level I/O values implementing `std::io::Read`/`Write` or `tokio::io::AsyncRead`/`AsyncWrite` (files, sockets, compression codecs).
- Fetch the docs pages of what you found. They list the supported libraries and versions; skip a primitive the docs do not support.

### 2. Add the dependency and feature passthrough

In the target crate's `Cargo.toml`:

```toml
[dependencies]
hotpath = "0.28"

[features]
hotpath = ["hotpath/hotpath"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
hotpath-prometheus = ["hotpath", "hotpath/hotpath-prometheus"]
hotpath-cloud = ["hotpath", "hotpath/hotpath-cloud"]
```

Enable the extra hotpath cargo features on the dependency (`tokio`, a channel or lock library, an SQL or HTTP integration, ...) that the pages fetched in step 1 name for the primitives the project uses. Pick the HTTP and axum features matching the project's major versions. For example: `hotpath = { version = "0.28", features = ["tokio"] }`.

Use this version, not the one the docs snippets show. If the project already depends on `hotpath`, keep its version.

If the crate already has a `[features]` section, merge the entries.

### 3. Instrument main

Docs: https://hotpath.rs/functions, https://hotpath.rs/profiling_modes.

- Put `#[hotpath::main]` on `main`. With tokio, `#[tokio::main]` must come FIRST (above it). Defaults are fine for a first setup; don't add parameters unless asked.
- If attribute placement on main is not possible, build a guard with `HotpathGuardBuilder`. The builder does not install the allocation-tracking allocator, so also declare `hotpath::CountingAllocator` as the `#[global_allocator]` ("Allocation tracking with `HotpathGuardBuilder`").
- If the project already declares a custom global allocator (jemalloc, mimalloc, ...), follow "Custom inner allocator": the `allocator = ...` parameter with the macro, `CountingAllocator::with(...)` with the builder. Never leave two `#[global_allocator]` statics active under `hotpath-alloc`.

### 4. Instrument functions

Docs: https://hotpath.rs/functions.

- Prefer `#[hotpath::measure_all]` on inline modules and `impl` blocks. Exclude noisy or trivial functions with `#[hotpath::skip]`.
- Use `#[hotpath::measure]` on individual functions, both sync and async, and `hotpath::measure_block!` for ad-hoc code blocks.
- Start with hot paths: request handlers, worker loops, parsing/serialization, IO-heavy functions. Don't instrument one-line getters.
- Don't try to instrument everything, use up to ~5 `hotpath::measure_all` annotations and up to 30 `hotpath::measure`. Goal of the initial setup is not to measure all functions, but to get the initial working instrumentation in place.

### 5. Wrap data-flow primitives

Docs: https://hotpath.rs/data_flow, https://hotpath.rs/locks, https://hotpath.rs/io_tracing.

Wrap channels, streams, futures, locks and I/O values at their creation site with the macro the docs give, with a `label`.

- Call-site aggregation is the default for `channel!`, `stream!` and `io!`, and it is safe for unbounded instance churn. Use `iter = true` only where one row per instance is wanted and the number of instances is bounded.
- For `io!`, wrap the side where the work happens ("Measured layers", "Compression encoders"), or the reported rate is meaningless.
- Wrapper macros return types prefixed with `hotpath::wrap`; update type signatures where needed. Explain to the user that these types are no-op unless the `hotpath` feature is enabled. If that requires changes across many function boundaries, note it to the user rather than rewriting half the codebase silently.
- Apply `log = true` only if `Debug` is already implemented.

### 6. Optional extras (only when relevant)

Fetch the page before adding one of these:

- Tokio runtime metrics: https://hotpath.rs/tokio_runtime.
- SQL profiling (sqlx, diesel, toasty): https://hotpath.rs/sql_tracing. Mind the "EnvFilter caveat".
- HTTP client profiling (reqwest, ureq): https://hotpath.rs/http_tracing. Wrap the client once at creation; where it is stored in a struct or named in signatures, use the `hotpath::wrap::` types. When both reqwest majors are enabled, 0.12 clients and errors use the versioned `hotpath::wrap::reqwest_012` path. If the app already uses reqwest-middleware, attach the hotpath middleware to its stack instead of the macro.
- axum server profiling: https://hotpath.rs/axum_tracing. Wrap the finished router, after the last route is added; with an existing tower stack, place the layer as "Existing middleware stacks" describes. Tell the user that route scoping of SQL queries and outbound HTTP requests is on by default and how to disable it. Async sqlx sqlite runs statements on its own worker thread, so it gets neither source nor route.

### 7. Add the policy file

```bash
mkdir -p hotpath
curl -fsSL https://hotpath.rs/api/v1/policy/default | jq -j .source > hotpath/policy.toml
```

Run it from the repository root. It writes the default regression policy, with comments, to `hotpath/policy.toml` (no token needed; with `wget`, use `wget -qO- https://hotpath.rs/api/v1/policy/default`). Never overwrite an existing policy file. Do not change the defaults unless the user asks; https://hotpath.rs/regression_policy documents every key.

### 8. Verify

```bash
cargo check                       # feature off: must still compile, zero overhead
cargo check --features hotpath    # feature on
cargo run --features hotpath      # prints report on exit
```

Optionally verify alloc mode: `cargo run --features 'hotpath-alloc'`.

Report what was instrumented and mention next steps, with their docs links: the live TUI and report options (https://hotpath.rs/profiling_modes) and the environment variables (https://hotpath.rs/configuration). Report sections need no configuration, so mention `HOTPATH_REPORT` only if the user wants to restrict output.

Also explain to the user that hotpath is safe to keep as a regular (non-optional) dependency ("Optional vs non-optional" in https://hotpath.rs/profiling_modes).

## Rules

- Never enable the `hotpath` feature by default (`default = []`); profiling must stay opt-in.
- Keep edits minimal: dependency, main, the policy file, and a sensible starting set of instrumented functions/primitives. Expand coverage only when the user asks.
- When the docs and your memory of the API disagree, the docs win.
