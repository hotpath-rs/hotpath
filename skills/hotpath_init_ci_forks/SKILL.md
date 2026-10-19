---
name: hotpath_init_ci_forks
description: Set up the hotpath Cloud CI integration in a public Rust project that benchmarks pull requests from forks. Adds the hotpath-cloud feature, a regression policy file, a GitHub Actions benchmark workflow that writes hotpath reports as artifacts and a relay workflow that uploads them, so every pull request, forks included, gets a performance comment. Use when the user wants to add hotpath Cloud to an open source project or benchmark pull requests from forks.
allowed-tools: Bash, Read, Edit, Write, Glob, Grep
---

# Initialize hotpath Cloud CI for pull requests from forks

Set up [hotpath Cloud](https://hotpath.rs/cloud) in the current Rust project, covering pull requests from forks. A GitHub Actions job runs a benchmark with hotpath profiling enabled. hotpath.rs compares each pull request's report with the default branch's, judges the difference under the repository's policy file and posts a comment on the pull request.

A job triggered by a fork's pull request cannot upload, so the pull request job stores the report as an artifact and a relay workflow uploads it from a job that never checks out or runs code from the pull request. "Pull requests from forks" in the docs explains the design.

This setup also covers pull requests from branches of the same repository. For a private repository, or one that does not need fork pull requests benchmarked, the simpler `hotpath_init_ci` skill is enough (`hotpath init-ci` without `--forks`).

## Documentation

The hotpath docs are the reference for both workflow files, every environment variable and every policy key; this skill only holds the procedure. Every page of https://hotpath.rs is served as markdown when requested with an `Accept: text/markdown` header:

```bash
curl -sL -H 'Accept: text/markdown' https://hotpath.rs/ci_integration
```

Before each step, fetch the pages it names and follow them instead of relying on memory. Keep `-L` (some pages redirect) and the header (there are no `.md` URLs). A `#fragment` is not sent to the server: fetch the whole page and find the heading.

The site documents the latest hotpath release, which can be newer than the version the project uses. Never change the project's `hotpath` version because the docs show a newer one. If the docs describe something that version does not have, tell the user it needs a newer hotpath instead of working around it.

| Page | Covers |
|---|---|
| https://hotpath.rs/ci_integration | setup, "Pull requests from forks" (the benchmark workflow, the relay workflow and its inputs, environment variables), several benchmarks, troubleshooting |
| https://hotpath.rs/regression_policy | the policy file: sections, keys, validation, the pull request comment and check |
| https://hotpath.rs/performance_budgets | absolute limits in the same policy file |
| https://hotpath.rs/cloud | what hotpath Cloud is |

## Steps

### 1. Inspect the project

- Find the crate(s), the default branch (`git symbolic-ref --short refs/remotes/origin/HEAD`, usually `main`) and the existing workflows in `.github/workflows/`.
- Check whether hotpath is already set up: a `hotpath` dependency, `#[hotpath::main]` (or `HotpathGuardBuilder`) and instrumented functions. If it is not, set it up first by following the `hotpath_init` skill (https://raw.githubusercontent.com/hotpath-rs/hotpath/main/skills/hotpath_init/SKILL.md), then continue here.

### 2. Pick the benchmark

The benchmark is a program instrumented with hotpath that runs a **fixed workload**: the same work on every run, so two runs of one commit report the same allocations. It runs to completion and exits normally, so the hotpath guard is dropped and the report is written.

- Prefer an existing example, binary or integration test that exercises the code the user cares about. Ask the user which one when it is not obvious.
- If there is none, propose a new example (`examples/hotpath_benchmark.rs`) that drives the main code paths with fixed inputs, and write it only after the user agrees.
- No randomness without a fixed seed, no network calls to services CI cannot reach, no timing-dependent loops. Enough calls per function that the numbers are stable.
- Pick the benchmark name, used as `HOTPATH_BENCHMARK` and as the relay's `benchmark` input: 1 to 64 characters from `[A-Za-z0-9._-]`, for example the example's name. An invalid name skips the upload.

### 3. Add the `hotpath-cloud` feature

Docs: https://hotpath.rs/ci_integration ("Setup").

Add the `hotpath-cloud` feature passthrough from the docs snippet to the benchmark crate's `Cargo.toml`, next to the existing hotpath features. Leave the `hotpath` dependency version as it is. Never add it to `default`. The pull request job needs it too, although it does not upload.

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

A pull request is judged under the policy file of its own branch, forks included. Tell the user to review changes to `hotpath/*.toml` in pull requests like changes to CI configuration.

### 5. Add the benchmark workflow

Docs: https://hotpath.rs/ci_integration ("Pull requests from forks", "The benchmark workflow").

Create `.github/workflows/hotpath-benchmark.yml` from the workflow in that section, copied as is, with its `upload` and `report` jobs. Then adapt only:

- `main`: the default branch.
- `name:`: it must be unique among the repository's workflows, since the relay matches the benchmark workflow by name. If another workflow already has that name, rename this one (e.g. `hotpath benchmark`).
- `my_benchmark`: the benchmark name, in `HOTPATH_BENCHMARK` and in the commands of both jobs.
- both `cargo run` lines: the command that runs the benchmark (`-p <crate>` in a workspace, `--bin` or `--example` as appropriate). Keep `--release`. Drop `hotpath-alloc` from `--features` only if the crate has no such feature.
- Add any setup the benchmark needs (system packages, a database service) to both jobs, before the benchmark, copied from the repository's existing CI.

Never set `HOTPATH_UPLOAD` or `id-token: write` in the `report` job: it runs untrusted code.

### 6. Add the relay workflow

Docs: https://hotpath.rs/ci_integration ("The relay workflow").

Create `.github/workflows/hotpath-relay.yml` from the workflow in that section, copied as is. Then check:

- The `workflows:` entry is exactly the `name:` of the benchmark workflow of step 5.
- `benchmark`, `artifact_name` and `report_file` match what the `report` job of step 5 uses.
- The relay is pinned to `hotpath-relay.yml@v0.28.6`, whatever tag the docs workflow shows, never `@main`. The pin must be the release tag of the `hotpath` version the benchmark builds with: if the project's `Cargo.lock` resolves another version, use that version's tag instead. If the repository pins its actions by commit hash, use the commit the tag points to (`git ls-remote https://github.com/hotpath-rs/hotpath refs/tags/v0.28.6`) with the tag in the comment: `@<commit sha> # v0.28.6`.

A regression never fails the job. If the user wants the pull request's `hotpath / <benchmark>` check to fail, or to block merges, follow "Failing the job" in https://hotpath.rs/ci_integration and "Pull request check" in https://hotpath.rs/regression_policy, and validate the policy after the edit.

Tell the user the "Things to know about `workflow_run`" of the docs: the relay runs from the default branch only, renaming the benchmark workflow silently stops it, and the relay run is not a check on the pull request.

### 7. Verify locally

```bash
cargo check                                                    # feature off still compiles
HOTPATH_BENCHMARK=my_benchmark HOTPATH_OUTPUT_FORMAT=json HOTPATH_OUTPUT_PATH=/tmp/hotpath-report.json \
  cargo run --release --example my_benchmark --features hotpath,hotpath-alloc,hotpath-cloud
jq '{benchmark: .meta.benchmark, policy: .meta.policy.path}' /tmp/hotpath-report.json
```

The report must name the benchmark and `hotpath/policy.toml`: this is the file the pull request job stores as the artifact. Run the benchmark twice and compare the function allocation totals: if they differ, the workload is not fixed, so fix that before relying on the comments.

### 8. Tell the user what is left

- Log in at https://hotpath.rs/app and install the hotpath GitHub App on the repository from the dashboard ("Install the hotpath app" or "Add repositories"). "Setup" in https://hotpath.rs/ci_integration lists the access it needs.
- Commit both workflows and merge them to the default branch. The first push to the default branch uploads the **baseline**, and the relay only runs once it is on the default branch. Until the baseline exists, pull request comments read "no baseline yet".
- From then on, every pull request, forks included, gets a performance comment. The `hotpath_cloud` skill and `hotpath cloud diff` read the same verdict as JSON.
- If an upload, a comment or the relay does not show up, the "Troubleshooting" table of https://hotpath.rs/ci_integration maps each symptom to its cause.

Do not commit or push unless the user asks.

## Rules

- Never enable `hotpath`, `hotpath-alloc` or `hotpath-cloud` by default; profiling stays opt-in.
- Never put a token or secret in either workflow; uploads authenticate with the OIDC token of trusted jobs.
- Never give the pull request job `id-token: write` or secrets, and never check out pull request code in the relay.
- One benchmark per `HOTPATH_BENCHMARK` name. For several benchmarks, add one job pair and one relay job each, with their own names and artifact names ("Several benchmarks in one repository" in https://hotpath.rs/ci_integration); a benchmark's own policy file is fetched the same way as `hotpath/policy.toml`.
- Keep the edits to the feature, the policy file, the two workflows and, if the user agreed, the benchmark program.
