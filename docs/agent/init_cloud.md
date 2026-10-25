# Set up hotpath Cloud CI in a Rust project

Instructions for an AI coding agent. Set up [hotpath Cloud](https://hotpath.rs/cloud) in the current Rust project. A GitHub Actions job runs a benchmark with hotpath profiling enabled and uploads the report. hotpath.rs compares each pull request's report with the default branch's, judges the difference under the repository's policy file and posts a comment on the pull request.

The benchmark runs in one of two profiling modes, picked in step 3:

- **Merged** (recommended for new benchmarks when suitable): native timing + Callgrind instruction counts in one report. One build runs twice in the job: natively, for timing, allocations and every other section, then under Valgrind's Callgrind, which merges the native report into its own and uploads it. Instruction counts measure computational work, not speed, and are reproducible for a deterministic workload, so changes in CPU work are judged without the noise of shared CI runners; the timing is shown next to them without being judged.
- **Native**: the benchmark runs natively once, and the report carries allocations and timing. Timing on shared runners is noisy, so it is shown without being judged.

A third mode, Callgrind only (instruction counts and allocations, no timing), exists in some repositories. Recognize and report it; never set it up.

There are two setups, picked in step 1:

- **Same repository**: pull requests come from branches of the repository itself (private repositories, or public ones that do not benchmark fork pull requests). One workflow runs the benchmark and uploads the report.
- **Forks**: a public repository that wants pull requests from forks benchmarked too. A job triggered by a fork's pull request cannot upload, so the pull request job stores the report as an artifact and a relay workflow uploads it from a job that never checks out or runs code from the pull request. "Pull requests from forks" in https://hotpath.rs/ci_integration explains the design. This setup also covers pull requests from branches of the same repository.

## Documentation

The hotpath docs are the reference for the workflow files, every environment variable and every policy key; this page only holds the procedure. Every page of https://hotpath.rs is served as markdown when requested with an `Accept: text/markdown` header:

```bash
curl -fsSL -H 'Accept: text/markdown' https://hotpath.rs/ci_integration
```

Before each step, fetch the pages it names and follow them instead of relying on memory. Keep `-fsSL` and the header. If a fetch fails, stop and tell the user which URL could not be retrieved; do not continue from memory. A `#fragment` is not sent to the server: fetch the whole page and find the heading.

The site documents the latest hotpath release, `{{HOTPATH_RELEASE_TAG}}` (the crate version is the tag without its `v`). When the project already depends on an older `hotpath`, update it to that release before following this procedure:

- Where the project's `hotpath` requirement (each crate's `Cargo.toml`, or `[workspace.dependencies]`) does not allow that version, raise it to `"{{HOTPATH_VERSION}}"`, keeping its features. Then run `cargo update -p hotpath` and check that `Cargo.lock` resolves the release.
- Build with and without the project's hotpath features (`cargo check`, and `cargo check --features` with the features the project defines) and fix what the update breaks, using the release notes in https://github.com/hotpath-rs/hotpath/blob/main/CHANGELOG.md.
- Tell the user you updated hotpath: the old and the new version, and any code you changed for it.
- If the update cannot be made to build, revert it, continue on the version the project had and tell the user why. When the docs then describe something that version does not have, tell the user instead of working around it.

| Page | Covers |
|---|---|
| https://hotpath.rs/ci_integration | setup, "Pull requests from the same repository", "Pull requests from forks" (the benchmark workflow, the relay workflow and its inputs), failing the job, environment variables, "Profiling modes" (merged, native, Callgrind only, switching an existing benchmark to merged), several benchmarks, troubleshooting |
| https://hotpath.rs/callgrind | the `hotpath-callgrind` feature: merged reports ("Timing and instruction counts in one report"), what is counted, reproducibility limits, comparing runs, build requirements |
| https://hotpath.rs/regression_policy | the policy file: sections, keys, validation, the pull request comment and check |
| https://hotpath.rs/performance_budgets | absolute limits in the same policy file |
| https://hotpath.rs/cloud | what hotpath Cloud is |

## Steps

### 1. Inspect the project

- Find the crate(s), the default branch (`git symbolic-ref --short refs/remotes/origin/HEAD`, usually `main`) and the existing workflows in `.github/workflows/`.
- Check whether hotpath Cloud is already set up: a workflow that sets `HOTPATH_UPLOAD` or calls `hotpath-relay.yml`. If it is:
  - Unless the user asked for another benchmark, do not set anything up. Report the existing setup instead: each workflow file and its `name:`, each `HOTPATH_BENCHMARK`, its profiling mode (merged when the Callgrind step sets `HOTPATH_MERGE_REPORT`, Callgrind only when the job runs `valgrind --tool=callgrind` without it, native otherwise), whether it uses the relay, the policy files in `hotpath/` and the README badge. Then stop.
  - If the user asked for another benchmark, continue to add only that one, with the setup (same repository or forks) the existing benchmarks use. Its benchmark name, workflow file, workflow `name:` and artifact name must all differ from the existing ones (steps 2 and 6).
  - Never change the profiling mode, the benchmark name, the workflow or the policy of an existing benchmark unless the user asks. If the user asks to switch a native or Callgrind-only benchmark to merged mode, check the requirements of step 3 and follow "Switching an existing benchmark to merged" in https://hotpath.rs/ci_integration.
- Check whether hotpath is already set up: a `hotpath` dependency, `#[hotpath::main]` (or `HotpathGuardBuilder`) and instrumented functions. If it is not, set it up first by following https://hotpath.rs/agent/setup (skip its "Offer hotpath Cloud" step), then continue here.
- Pick the setup, unless an existing benchmark already decides it. Check whether the repository is public or private (`gh repo view --json visibility`). A private repository uses the **same repository** setup. For a public repository, ask the user whether pull requests from forks should be benchmarked: if yes, use the **forks** setup, otherwise the **same repository** one, and tell them fork pull requests will then not get a comment.

### 2. Pick the benchmark

The benchmark is a program instrumented with hotpath that runs a **fixed workload**: the same work on every run, so two runs of one commit report the same allocations. It runs to completion and exits normally, so the hotpath guard is dropped and the report is written.

- Prefer an existing example, binary or integration test that exercises the code the user cares about. Ask the user which one when it is not obvious.
- If there is none, propose a new example (`examples/hotpath_benchmark.rs`) and write it only after the user agrees. A useful one:
  - drives the project's main work through its public API (the library's core operations, a server's main request flow, a CLI's main command), with realistic input sizes taken from existing tests, fixtures or `benches/`;
  - covers the main code paths with several distinct measured functions, so a regression points at one of them; add `#[hotpath::measure]` to the functions on those paths;
  - spends its time in the project's code, not in generating inputs or setup, and passes results through `std::hint::black_box` so the work is not optimized away;
  - finishes in a few seconds natively.
- No randomness without a fixed seed, no network calls to services CI cannot reach, no timing-dependent loops. Enough calls per function that the numbers are stable.
- Merged mode runs the benchmark twice in one job, so the second run must do the same work as the first: nothing the first run leaves behind (a database it fills, files it creates, a cache that changes its path) may change the second. Reset such state at the start of the program, or create it from scratch every run.
- Pick the benchmark name, used as `HOTPATH_BENCHMARK` (and, in the forks setup, as the relay's `benchmark` input): 1 to 64 characters from `[A-Za-z0-9._-]`, for example the example's name. An invalid name skips the upload. It must not be used by any existing workflow: series are separate only by name.

### 3. Pick the profiling mode

Docs: https://hotpath.rs/ci_integration ("Profiling modes"), https://hotpath.rs/callgrind ("Timing and instruction counts in one report").

Use **merged** when all of these hold, otherwise **native**:

- The `hotpath` version in `Cargo.lock` is `0.29.2` or newer (it can merge reports). The update at the start of this procedure provides it; if that update had to be reverted, use native mode and tell the user that merged mode needs `hotpath` `0.29.2` or newer.
- The benchmark can run on the docs workflow's x86_64 Linux runner: it does not need macOS, Windows or an ARM runner.
- The workload is CPU-bound and deterministic: its time goes into computation, not into network, disk, sleeps, timers or a database service, and it is not dominated by many threads racing each other (Valgrind serializes threads, so their interleaving moves the counts).
- The code it should judge is in functions with `#[hotpath::measure]` (or `measure_all`); `measure_block!` blocks are not counted.
- `[profile.release]` does not strip symbols (`strip = true` or `"symbols"`).
- It finishes under Valgrind within the job timeout. Valgrind runs it tens of times slower: a workload that takes more than about a minute natively should be made smaller or run natively.
- It runs twice in one job doing the same work (step 2).

Never pick Callgrind only for a new benchmark. Merged mode is preferred, not required. Tell the user which mode you picked and why, in one or two sentences. A benchmark that turns out too slow, not repeatable or not reproducible under Valgrind in step 7 falls back to native: then undo the merged-only parts of steps 4 to 6 (the `hotpath-callgrind` feature, the `[functions.icount]` edit or the benchmark's own policy file, and the merged workflow, replaced by the native one).

### 4. Add the features

Docs: https://hotpath.rs/ci_integration ("Setup", and "Profiling modes").

Add the `hotpath-cloud` feature passthrough from the docs snippet to the benchmark crate's `Cargo.toml`, next to the existing hotpath features, unless it is already there. For merged mode add the `hotpath-callgrind` passthrough as well, and keep `hotpath-alloc`. Keep the `hotpath` version the update at the start of this procedure set. Never add them to `default`. In the forks setup, the pull request job needs them too, although it does not upload.

### 5. Add the policy file

Docs: https://hotpath.rs/regression_policy ("Which file a run uses", "Functions instructions").

Every benchmark must have a policy file, or its uploads are refused. A run uses `hotpath/<benchmark>-policy.toml` when it exists, otherwise the shared `hotpath/policy.toml` (and only the file `HOTPATH_POLICY_PATH` names, when a workflow sets it). Pick the case that applies:

| Policy files the new benchmark would use | New native benchmark | New merged benchmark |
|---|---|---|
| neither | write `hotpath/policy.toml` (below) | write `hotpath/policy.toml` (below), then set `[functions.icount]` in it |
| `hotpath/policy.toml` only | keep it, nothing to add | keep it if it already has a `judged = true` `[functions.icount]` (an existing merged or Callgrind benchmark uses it); otherwise create `hotpath/<benchmark>-policy.toml` as a copy of it, then set `[functions.icount]` in the copy |
| `hotpath/<benchmark>-policy.toml` | keep it | keep it; if it has no `[functions.icount]`, show the user the block below and add it only if they agree |

A fresh repository has no policy file:

```bash
mkdir -p hotpath
if [ -e hotpath/policy.toml ]; then
  echo "hotpath/policy.toml exists, keeping it"
else
  curl -fsSL https://hotpath.rs/api/v1/policy/default | jq -je .source > hotpath/policy.toml.tmp \
    && mv hotpath/policy.toml.tmp hotpath/policy.toml \
    || { rm -f hotpath/policy.toml.tmp; echo "fetching the default policy failed"; false; }
fi
```

Run it from the repository root. It writes the default policy, with comments, to `hotpath/policy.toml` (no token needed; with `wget`, use `wget -qO- https://hotpath.rs/api/v1/policy/default`). It is the same file `hotpath cloud init` of the docs writes, without needing the CLI. Every upload must carry a policy file, so commit it. Do not change the defaults unless the user asks, except for merged mode below.

For a merged benchmark next to an existing shared policy, copy it so the new benchmark keeps the repository's `pr_check`, thresholds and budgets, and the shared file and the benchmarks it serves stay as they are:

```bash
if [ -e hotpath/my_benchmark-policy.toml ]; then
  echo "hotpath/my_benchmark-policy.toml exists, keeping it"
else
  cp hotpath/policy.toml hotpath/my_benchmark-policy.toml
fi
```

Never overwrite an existing policy file, shared or per benchmark: keep the existence checks when you adapt the commands. Tell the user the new file is now the benchmark's whole policy: later edits to `hotpath/policy.toml` no longer reach it.

`[functions.icount]` judges the instruction counts. The default policy already lists it with the built-in 2; in the file the new merged benchmark uses (written or copied above, not one that existed before), set it to this conservative starting point, adding the section if it is missing, and keep the other sections as they are:

```toml
[functions.icount]
judged = true
fail_check = false
min_percent_change = 3
metrics = ["avg", "total"]
min_calls = 1
```

A deterministic single-threaded benchmark may support the built-in 2 later; how much a concurrent one varies depends on the workload, so step 7 measures the spread and the threshold must sit above it. A section the file leaves out is not compared.

A merged report also carries native timing. Keep `[functions.timing]` listed with `judged = false`, as the default policy writes it, so the timing shows next to the counts on the comparison page and in the pull request comment; never make it judged unless the user asks, since timing on shared runners is noisy.

Validate every policy file you wrote, copied or edited, with its own path in place of `hotpath/policy.toml`. A valid policy answers `{"valid":true}`, an invalid one HTTP 422 with `problems`, each with its `line` and `message`:

```bash
jq -Rs '{source: .}' hotpath/policy.toml \
  | curl -sS -X POST https://hotpath.rs/api/v1/policy/validate -H 'Content-Type: application/json' --data-binary @-
```

In the forks setup, a pull request is judged under the policy file of its own branch, forks included. Tell the user to review changes to `hotpath/*.toml` in pull requests like changes to CI configuration.

### 6. Add the workflows

Follow the section of the setup picked in step 1. Never overwrite an existing workflow file: if the file name below is taken, or a workflow with the same `name:` exists, use `hotpath-<benchmark>.yml` and `name: hotpath <benchmark>` instead, and ask the user if that is taken too. The setup sections of https://hotpath.rs/ci_integration show the merged workflows; copy them as is for merged mode, keeping in every job: the pinned runner label, Valgrind package and Rust toolchain (never `ubuntu-latest` or `stable`), one `cargo build` with `--features hotpath-alloc,hotpath-cloud,hotpath-callgrind` and the same binary run twice, the job-level `env` with static values (the benchmark name, `HOTPATH_USER_METADATA`, `HOTPATH_OUTPUT_FORMAT: json`), `HOTPATH_UPLOAD`, `HOTPATH_MERGE_REPORT` and the output paths only on their step, `HOTPATH_CALLGRIND_OUT_FILE` equal to `--callgrind-out-file`, the `jq` check and the longer timeout. For native mode use "Native" under "Profiling modes" instead (in the forks setup, both jobs in that shape).

Pin the Rust toolchain to an exact version in every job that builds the benchmark, for both profiling modes and in both jobs of the forks setup: replace `dtolnay/rust-toolchain@stable` with `dtolnay/rust-toolchain@master` and `toolchain: "<x.y.z>"`, and write the same version into `toolchain=` of `HOTPATH_USER_METADATA`. Never leave `stable`, `beta`, `nightly` or a bare `x.y`: a new Rust release changes the numbers, so the first pull request after it would be judged against a baseline built with the older `rustc` and flagged for changes it did not cause. Use the version the repository's `rust-toolchain.toml` or existing CI pins, otherwise the one the docs merged workflow shows. Tell the user to bump it in a separate pull request, whose merge uploads the new baseline.

#### Same repository

Docs: https://hotpath.rs/ci_integration ("Pull requests from the same repository").

Create `.github/workflows/hotpath-benchmark.yml` (or the unique name above) from the workflow in that section, copied as is. Then adapt only:

- `main`: the default branch.
- `name:`: unique among the repository's workflows.
- `my_benchmark`: the benchmark name, in `HOTPATH_BENCHMARK` and in the command.
- the `cargo build` line and both binary paths (for native mode, the `cargo run` line): the command that builds and runs the benchmark (`-p <crate>` in a workspace, `--bin` or `--example` as appropriate). Keep `--release`. Drop `hotpath-alloc` from `--features` only if the crate has no such feature.
- Add any setup the benchmark needs (system packages, a database service) as steps before the benchmark, copied from the repository's existing CI.

Keep the `test -s` line, the `jq` check, the absolute response path, the job-level `env` and `HOTPATH_UPLOAD: fail-on-error`; the docs explain what each guards against. The workflow needs no secret.

#### Forks: the benchmark workflow

Docs: https://hotpath.rs/ci_integration ("Pull requests from forks", "The benchmark workflow").

Create `.github/workflows/hotpath-benchmark.yml` (or the unique name above) from the workflow in that section, copied as is, with its `upload` and `report` jobs. Then adapt only:

- `main`: the default branch.
- `name:`: it must be unique among the repository's workflows, since the relay matches the benchmark workflow by name. If another workflow already has that name, rename this one (e.g. `hotpath benchmark`).
- `my_benchmark`: the benchmark name, in `HOTPATH_BENCHMARK` and in the commands of both jobs.
- `hotpath-report`: the artifact name. Keep it for the first benchmark; an additional one uses `hotpath-report-<benchmark>`.
- the `cargo build` line and the binary paths of both jobs (for native mode, both `cargo run` lines): the command that builds and runs the benchmark (`-p <crate>` in a workspace, `--bin` or `--example` as appropriate). Keep `--release`. Drop `hotpath-alloc` from `--features` only if the crate has no such feature.
- Add any setup the benchmark needs (system packages, a database service) to both jobs, before the benchmark, copied from the repository's existing CI.

Never set `HOTPATH_UPLOAD` or `id-token: write` in the `report` job: it runs untrusted code. Merged mode does not change its permissions: the `report` job keeps `contents: read` alone and the checkout of the pull request's head commit.

#### Forks: the relay workflow

Docs: https://hotpath.rs/ci_integration ("The relay workflow").

Every benchmark workflow gets its own relay. Create `.github/workflows/hotpath-relay.yml` (for an additional benchmark, or when the name is taken, `hotpath-relay-<benchmark>.yml`, with a unique `name:`) from the workflow in that section, copied as is. Never edit an existing relay to point at the new benchmark. Then check:

- The `workflows:` entry is exactly the `name:` of this benchmark's workflow.
- `benchmark`, `artifact_name` and `report_file` match what the `report` job of the benchmark workflow uses.
- The relay is always pinned by the full commit SHA of a release tag, with the tag in a comment, never by the bare tag and never `@main`: the relay job holds `id-token: write`, and a tag can be moved. The tag is `{{HOTPATH_RELEASE_TAG}}`, whatever tag the docs workflow shows, unless the project's `Cargo.lock` resolves another `hotpath` version: then use that version's tag, so the relay matches the release the benchmark builds with. Resolve the commit with `git ls-remote https://github.com/hotpath-rs/hotpath 'refs/tags/{{HOTPATH_RELEASE_TAG}}*'` (if a `^{}` line is listed, take its SHA, the commit an annotated tag points to) and replace the docs' `@{{HOTPATH_RELEASE_TAG}}` with it:

  ```yaml
      uses: hotpath-rs/hotpath/.github/workflows/hotpath-relay.yml@<40-character commit sha> # {{HOTPATH_RELEASE_TAG}}
  ```

  Never shorten the SHA or guess it: if `git ls-remote` fails or lists no such tag, stop and ask the user.

Tell the user the "Things to know about `workflow_run`" of the docs: the relay runs from the default branch only, renaming the benchmark workflow silently stops it, and the relay run is not a check on the pull request.

#### Failing the job

A regression never fails the job. If the user wants the pull request's `hotpath / <benchmark>` check to fail, or to block merges, follow "Failing the job" in https://hotpath.rs/ci_integration and "Pull request check" in https://hotpath.rs/regression_policy, and validate the policy after the edit.

### 7. Verify locally

Run each verification below as its own script (`bash -s` with the block on stdin, or a file created with `mktemp`), so `set -euo pipefail` and the cleanup trap end with it instead of changing your shell. Every attempt writes its reports into its own `mktemp -d` directory, never to fixed paths, so a report from an earlier attempt is never read as this one's, and removes only that directory, whether the attempt succeeds or fails.

```bash
set -euo pipefail

cargo check                                                    # feature off still compiles

verify_dir="$(mktemp -d)"
trap 'rm -rf -- "$verify_dir"' EXIT

HOTPATH_BENCHMARK=my_benchmark HOTPATH_OUTPUT_FORMAT=json HOTPATH_OUTPUT_PATH="$verify_dir/report.json" \
  cargo run --release --example my_benchmark --features hotpath-alloc,hotpath-cloud
jq '{benchmark: .meta.benchmark, policy: .meta.policy.path, run: .meta.run}' "$verify_dir/report.json"
```

For merged mode, add `hotpath-callgrind` to `--features`: run natively, the same build prints the usual timing report.

The report must name the benchmark and the policy file step 5 picked for it (`hotpath/<benchmark>-policy.toml` when that exists, otherwise `hotpath/policy.toml`). Run the benchmark twice and compare the function allocation totals: if they differ, the workload is not fixed, so fix that before relying on the comments.

For merged mode on an x86_64 Linux machine with Valgrind installed, also run the merged pair the way the workflow does: one build, the same binary run natively, then under Callgrind only after the native run succeeded:

```bash
set -euo pipefail

cargo build --release --example my_benchmark \
  --features hotpath-alloc,hotpath-cloud,hotpath-callgrind

verify_dir="$(mktemp -d)"
trap 'rm -rf -- "$verify_dir"' EXIT

# Native run (must succeed)
HOTPATH_BENCHMARK=my_benchmark HOTPATH_OUTPUT_FORMAT=json \
  HOTPATH_OUTPUT_PATH="$verify_dir/native.json" \
  target/release/examples/my_benchmark

# Callgrind run, merging the native report (must succeed)
HOTPATH_BENCHMARK=my_benchmark HOTPATH_OUTPUT_FORMAT=json \
  HOTPATH_OUTPUT_PATH="$verify_dir/merged.json" \
  HOTPATH_MERGE_REPORT="$verify_dir/native.json" \
  HOTPATH_CALLGRIND_OUT_FILE="$verify_dir/callgrind.out.%p" \
  valgrind -q --tool=callgrind --instr-atstart=no \
    --callgrind-out-file="$verify_dir/callgrind.out.%p" \
    target/release/examples/my_benchmark

# jq verification (must succeed), then the counts to compare between attempts
jq -e '.meta.run == "merged" and (.functions_icount.data | length > 0) and (.functions_timing.data | length > 0)' \
  "$verify_dir/merged.json" > /dev/null
jq '{tool: .functions_icount.tool, functions: [.functions_icount.data[] | {name, calls, total}]}' \
  "$verify_dir/merged.json"
```

The script stops at the first failing command (the build, either run, or the `jq -e` check) with a nonzero status. `functions_icount.data` must list the measured functions and `tool` the Valgrind version. A `merge failed: ...` line names what the two runs disagree on or why the counts are missing: fix that rather than dropping the merge. Run the script twice, each attempt with its own directory, and compare the printed totals: they should differ by well under the policy's `min_percent_change`. If they differ by more, raise it above the observed spread in the policy file step 5 wrote or copied (ask before editing a file that existed before), or fall back to native when the counts are not reproducible or the run is too slow for CI. Tell the user what you measured.

Without x86_64 Linux and Valgrind, do not install Valgrind or set up a container for this; tell the user that the merged reports are verified by the first CI runs, and that re-running the default branch's job should give the same instruction counts.

### 8. Add the README badge

Docs: https://hotpath.rs/cloud ("README badge").

Add the hotpath Cloud badge to the README that GitHub renders for the repository (the repository root `README.md`, not a member crate's README in a workspace), linking to the repository's page on hotpath.rs. Take `OWNER/NAME` from `gh repo view --json nameWithOwner -q .nameWithOwner`, or from the `origin` remote:

```markdown
[![hotpath-cloud](https://img.shields.io/endpoint?url=https://hotpath.rs/badge-cloud.json)](https://hotpath.rs/app/repos/OWNER/NAME)
```

- If the README already contains `badge-cloud.json`, leave it as it is. One badge covers every benchmark of the repository.
- Put the badge at the end of an existing badge line, otherwise on its own line below the title. Change nothing else in the README.
- If the repository has no README, give the user the snippet instead of creating one.

### 9. Tell the user what is left

- Log in at https://hotpath.rs/app and install the hotpath GitHub App on the repository from the dashboard ("Install the hotpath app" or "Add repositories"). "Setup" in https://hotpath.rs/ci_integration lists the access it needs.
- Commit the workflow(s) and the policy file and merge them to the default branch. Pull request branches older than that commit have no policy file, so their uploads are refused until the default branch is merged into them. The first push to the default branch uploads the **baseline** (and the relay of the forks setup only runs once it is on the default branch). Until the baseline exists, pull request comments read "no baseline yet".
- The badge links to the repository's page on hotpath.rs, which shows "not found" until the GitHub App is installed. Reports of a public repository are public by default; for a private repository the page opens only for users with access to it.
- From then on, every pull request gets a performance comment. `hotpath cloud diff` reads the same verdict as JSON (https://hotpath.rs/agents_cli).
- If an upload, a comment or the relay does not show up, the "Troubleshooting" table of https://hotpath.rs/ci_integration maps each symptom to its cause.

Do not commit or push unless the user asks.

## Rules

- Never enable `hotpath`, `hotpath-alloc`, `hotpath-cloud` or `hotpath-callgrind` by default; profiling stays opt-in.
- Never put a token or secret in a workflow; uploads authenticate with the OIDC token of trusted jobs.
- In the forks setup, never give the pull request job `id-token: write` or secrets, and never check out pull request code in the relay.
- One benchmark per `HOTPATH_BENCHMARK` name, workflow file, workflow `name:`, artifact and relay. For several benchmarks, follow "Several benchmarks in one repository" in https://hotpath.rs/ci_integration.
- Never overwrite an existing workflow, policy file or badge. Running this procedure again on a configured repository reports the setup (step 1) instead of adding anything.
- Keep the edits to the feature, the policy file, the workflow(s), the README badge and, if the user agreed, the benchmark program. Leave unrelated workflows and repository settings alone.
- Do not install the GitHub App, log in to hotpath.rs or trigger an upload on the user's behalf; tell them the steps instead.
