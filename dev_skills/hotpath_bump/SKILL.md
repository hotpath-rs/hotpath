---
name: hotpath_bump
description: Bump the hotpath version number across the workspace and related files. Updates the versions of all six published crates (drain included) in Cargo.toml files (exact patch version), version references in the backend middleware and README (major.minor only), and the relay workflow release tag pins (exact `vX.Y.Z` tag). Use when the user wants to bump, bump the version, or release a new hotpath version.
allowed-tools: Bash, Read, Edit, Grep, Glob
---

# Bump hotpath Version

Bump the hotpath version number in every place it appears. The user provides the new version (e.g. `0.25.0`); if only `major.minor` is given, assume patch `0`.

Three formats are used:

- **Exact version** (`X.Y.Z`, e.g. `0.25.0`) - all Cargo.toml entries
- **Major.minor only** (`X.Y`, e.g. `0.25`) - crate version references
- **Release tag** (`vX.Y.Z`, e.g. `v0.25.0`) - `hotpath-relay.yml` pins

## 1. Cargo.toml files - exact version `X.Y.Z`

Update the `version` field in the `[package]` section of each crate:

- `crates/hotpath/Cargo.toml`
- `crates/hotpath-macros/Cargo.toml`
- `crates/hotpath-meta/Cargo.toml`
- `crates/hotpath-macros-meta/Cargo.toml`
- `crates/hotpath-drain/Cargo.toml`
- `crates/hotpath-drain-meta/Cargo.toml`

Update the `version` in the six `[workspace.dependencies]` entries in the root `Cargo.toml`:

```toml
hotpath-macros = { path = "./crates/hotpath-macros", version = "X.Y.Z" }
hotpath-drain = { path = "./crates/hotpath-drain", version = "X.Y.Z" }
hotpath = { path = "./crates/hotpath", version = "X.Y.Z" }
hotpath-meta = { path = "./crates/hotpath-meta", version = "X.Y.Z" }
hotpath-macros-meta = { path = "./crates/hotpath-macros-meta", version = "X.Y.Z" }
hotpath-drain-meta = { path = "./crates/hotpath-drain-meta", version = "X.Y.Z" }
```

All six crates always share the same version; never bump one without the others. This matters for the drain crates even when their code is unchanged: published `hotpath-drain` depends on `hotpath-meta`, so a drain left on an older release keeps pulling a stale `hotpath-meta` copy into downstream lockfiles (and splits meta profiling across two versions). All six are published together via `just cargo-publish`, which lists them in dependency order.

## 2. Other references - major.minor `X.Y`

- `../hotpath-backend/src/config/middleware.rs` (separate repo, sibling directory):

  ```rust
  ("{{HOTPATH_VERSION}}", "X.Y"),
  ```

- `README.md` - every version occurrence (`cargo install hotpath --version '^X.Y'`, `hotpath = "X.Y"`)

The docs and the agent pages in `docs/agent/` use the `{{HOTPATH_VERSION}}` and `{{HOTPATH_RELEASE_TAG}}` placeholders, which the backend fills from its `middleware.rs` entries above and below; never hardcode a version there.

## 3. Relay release tag - `vX.Y.Z`

Adopters pin the reusable relay workflow to the release tag, so these point at the tag this release creates:

- `../hotpath-backend/src/config/middleware.rs`:

  ```rust
  ("{{HOTPATH_RELEASE_TAG}}", "vX.Y.Z"),
  ```

- `.github/workflows/hotpath-relay.yml` - the `hotpath-relay.yml@vX.Y.Z` pin in the usage comment at the top

Leave `.github/workflows/cloud-random-benchmark-relay.yml` on `@main`: it tests the relay on the default branch.

## 4. Verify

Search for stragglers with the old version, excluding lockfiles, target dirs, and third-party code:

```bash
rg -n 'OLD_X\.OLD_Y' Cargo.toml README.md crates/hotpath/Cargo.toml crates/hotpath-macros/Cargo.toml crates/hotpath-meta/Cargo.toml crates/hotpath-macros-meta/Cargo.toml crates/hotpath-drain/Cargo.toml crates/hotpath-drain-meta/Cargo.toml
rg -n 'OLD_X\.OLD_Y' ../hotpath-backend/src/config/middleware.rs .github/workflows/hotpath-relay.yml
```

Ignore matches that are not hotpath version references (other dependencies may coincidentally share the number). Then run `cargo check` so `Cargo.lock` picks up the new versions.

## Rules

- Do not commit; leave changes for the user to review.
- Do not bump versions of test crates or any other dependency versions.
