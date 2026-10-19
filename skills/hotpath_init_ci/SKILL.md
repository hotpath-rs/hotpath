---
name: hotpath_init_ci
description: Set up the hotpath Cloud CI integration in a Rust project whose pull requests come from branches of the same repository (private repositories, or public ones that do not benchmark fork pull requests). Adds the hotpath-cloud feature, a regression policy file and a GitHub Actions benchmark workflow that uploads hotpath reports, so every pull request gets a performance comment. Use when the user wants to add hotpath Cloud, benchmark pull requests in CI or set up performance regression checks.
allowed-tools: Bash, Read, Edit, Write, Glob, Grep
---

# Initialize hotpath Cloud CI

Set up [hotpath Cloud](https://hotpath.rs/cloud) in the current Rust project. A GitHub Actions job runs a benchmark with hotpath profiling enabled and uploads the report. hotpath.rs compares each pull request's report with the default branch's, judges the difference under the repository's policy file and posts a comment on the pull request.

This skill covers pull requests from branches of the same repository. For a public repository that wants to benchmark pull requests from forks, use the `hotpath_init_ci_forks` skill instead (`hotpath init-ci --forks`).

## Documentation

The hotpath docs are the reference for the workflow file, every environment variable and every policy key; this skill only holds the procedure. Every page of https://hotpath.rs is served as markdown when requested with an `Accept: text/markdown` header:

```bash
curl -sL -H 'Accept: text/markdown' https://hotpath.rs/ci_integration
```

Before each step, fetch the pages it names and follow them instead of relying on memory. Keep `-L` (some pages redirect) and the header (there are no `.md` URLs). A `#fragment` is not sent to the server: fetch the whole page and find the heading.

The site documents the latest hotpath release, which can be newer than the version the project uses. Never change the project's `hotpath` version because the docs show a newer one. If the docs describe something that version does not have, tell the user it needs a newer hotpath instead of working around it.

| Page | Covers |
|---|---|
| https://hotpath.rs/ci_integration | setup, the workflow ("Pull requests from the same repository"), failing the job, environment variables, several benchmarks, troubleshooting |
| https://hotpath.rs/regression_policy | the policy file: sections, keys, validation, the pull request comment and check |
| https://hotpath.rs/performance_budgets | absolute limits in the same policy file |
| https://hotpath.rs/cloud | what hotpath Cloud is |

## Steps

### 1. Inspect the project

- Find the crate(s), the default branch (`git symbolic-ref --short refs/remotes/origin/HEAD`, usually `main`) and the existing workflows in `.github/workflows/`.
- Check whether hotpath is already set up: a `hotpath` dependency, `#[hotpath::main]` (or `HotpathGuardBuilder`) and instrumented functions. If it is not, set it up first by following the `hotpath_init` skill (https://raw.githubusercontent.com/hotpath-rs/hotpath/main/skills/hotpath_init/SKILL.md), then continue here.
- Check whether the repository is public or private (`gh repo view --json visibility`). For a public repository that accepts pull requests from forks, tell the user that fork pull requests will not be benchmarked by this setup and suggest `hotpath init-ci --forks`. Continue only if they confirm.

### 2. Pick the benchmark

The benchmark is a program instrumented with hotpath that runs a **fixed workload**: the same work on every run, so two runs of one commit report the same allocations. It runs to completion and exits normally, so the hotpath guard is dropped and the report is uploaded.

- Prefer an existing example, binary or integration test that exercises the code the user cares about. Ask the user which one when it is not obvious.
- If there is none, propose a new example (`examples/hotpath_benchmark.rs`) that drives the main code paths with fixed inputs, and write it only after the user agrees.
- No randomness without a fixed seed, no network calls to services CI cannot reach, no timing-dependent loops. Enough calls per function that the numbers are stable.
- Pick the benchmark name, used as `HOTPATH_BENCHMARK`: 1 to 64 characters from `[A-Za-z0-9._-]`, for example the example's name. An invalid name skips the upload.

### 3. Add the `hotpath-cloud` feature

Docs: https://hotpath.rs/ci_integration ("Setup").

Add the `hotpath-cloud` feature passthrough from the docs snippet to the benchmark crate's `Cargo.toml`, next to the existing hotpath features. Leave the `hotpath` dependency version as it is. Never add it to `default`.

### 4. Add the policy file

```bash
mkdir -p hotpath
curl -fsSL https://hotpath.rs/api/v1/policy/default | jq -j .source > hotpath/policy.toml
```

Run it from the repository root. It writes the default policy, with comments, to `hotpath/policy.toml` (no token needed; with `wget`, use `wget -qO- https://hotpath.rs/api/v1/policy/default`). It is the same file `hotpath cloud init` of the docs writes, without needing the CLI. Never overwrite an existing policy file. Every upload must carry a policy file, so commit it. Do not change the defaults unless the user asks; https://hotpath.rs/regression_policy documents every key.

After any edit, validate the file. A valid policy answers `{"valid":true}`, an invalid one HTTP 422 with `problems`, each with its `line` and `message`:

```bash
jq -Rs '{source: .}' hotpath/policy.toml \
  | curl -sS -X POST https://hotpath.rs/api/v1/policy/validate -H 'Content-Type: application/json' --data-binary @-
```

### 5. Add the workflow

Docs: https://hotpath.rs/ci_integration ("Pull requests from the same repository").

Create `.github/workflows/hotpath-benchmark.yml` from the workflow in that section, copied as is. Then adapt only:

- `main`: the default branch.
- `my_benchmark`: the benchmark name, in `HOTPATH_BENCHMARK` and in the command.
- the `cargo run` line: the command that runs the benchmark (`-p <crate>` in a workspace, `--bin` or `--example` as appropriate). Keep `--release`. Drop `hotpath-alloc` from `--features` only if the crate has no such feature.
- Add any setup the benchmark needs (system packages, a database service) as steps before the benchmark, copied from the repository's existing CI.

Keep the `test -s` line, the absolute response path and `HOTPATH_UPLOAD: fail-on-error`; the docs explain what each guards against. The workflow needs no secret.

A regression never fails the job. If the user wants the pull request's `hotpath / <benchmark>` check to fail, or to block merges, follow "Failing the job" in https://hotpath.rs/ci_integration and "Pull request check" in https://hotpath.rs/regression_policy, and validate the policy after the edit.

### 6. Verify locally

```bash
cargo check                                                    # feature off still compiles
HOTPATH_BENCHMARK=my_benchmark HOTPATH_OUTPUT_FORMAT=json HOTPATH_OUTPUT_PATH=/tmp/hotpath-report.json \
  cargo run --release --example my_benchmark --features hotpath,hotpath-alloc,hotpath-cloud
jq '{benchmark: .meta.benchmark, policy: .meta.policy.path}' /tmp/hotpath-report.json
```

The report must name the benchmark and `hotpath/policy.toml`. Run the benchmark twice and compare the function allocation totals: if they differ, the workload is not fixed, so fix that before relying on the comments.

### 7. Tell the user what is left

- Log in at https://hotpath.rs/app and install the hotpath GitHub App on the repository from the dashboard ("Install the hotpath app" or "Add repositories"). "Setup" in https://hotpath.rs/ci_integration lists the access it needs.
- Commit the changes and merge them to the default branch. The first push to the default branch uploads the **baseline**; until then, pull request comments read "no baseline yet".
- From then on, every pull request gets a performance comment. The `hotpath_cloud` skill and `hotpath cloud diff` read the same verdict as JSON.
- If an upload or a comment does not show up, the "Troubleshooting" table of https://hotpath.rs/ci_integration maps each message to its cause.

Do not commit or push unless the user asks.

## Rules

- Never enable `hotpath`, `hotpath-alloc` or `hotpath-cloud` by default; profiling stays opt-in.
- Never put a token or secret in the workflow; the upload needs none.
- One benchmark per `HOTPATH_BENCHMARK` name. For several benchmarks, follow "Several benchmarks in one repository" in https://hotpath.rs/ci_integration; a benchmark's own policy file is fetched the same way as `hotpath/policy.toml`.
- Keep the edits to the feature, the policy file, the workflow and, if the user agreed, the benchmark program.
