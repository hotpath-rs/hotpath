# CI integration: benchmark Rust pull requests in GitHub Actions

`hotpath Cloud` reviews the performance of every pull request. Your CI job runs a benchmark with `hotpath` profiling enabled and uploads the report. hotpath.rs compares it with the report of the base branch and posts the result as a comment and a `hotpath / <benchmark>` check on the pull request.

What counts as a regression is customizable with a [regression policy](regression_policy.md) and [performance budgets](performance_budgets.md).

## Setup

Log in at <a href="https://hotpath.rs/app" target="_blank" rel="noopener noreferrer">hotpath.rs/app</a> and install the hotpath GitHub App on the repository from the dashboard. The App has no access to your code, only write access to pull requests to post comments, and to checks to report the result of each benchmark.

Add the `hotpath-cloud` feature next to the profiling features you use:

```toml
[dependencies]
hotpath = "{{HOTPATH_VERSION}}"

[features]
hotpath = ["hotpath/hotpath"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
hotpath-cloud = ["hotpath", "hotpath/hotpath-cloud"]
```

Add a policy file. Every benchmark needs one: its own `hotpath/<benchmark>-policy.toml`, or the shared `hotpath/policy.toml` when it has none. An upload without one is refused. `hotpath cloud init` writes the default policy to `hotpath/policy.toml`:

```bash
cargo install hotpath --features cloud
hotpath cloud init
```

Commit the file. The [regression policy](regression_policy.md) page explains what it contains and how to change it.

The benchmark is any program of yours instrumented with `hotpath`: an example, a binary or an integration test that runs a fixed workload. The report is uploaded when the `hotpath` guard is dropped, at the end of the run.

There are two ways to wire the workflow, depending on where pull requests come from:

| Pull requests come from | Setup |
|---|---|
| branches of the same repository | [one workflow](#pull-requests-from-the-same-repository) that uploads from the benchmark job |
| forks (public repositories) | [two workflows](#pull-requests-from-forks): the benchmark job writes the report, a relay job uploads it |

The fork setup also handles pull requests from the same repository, so a project that accepts both needs only that one.

On x86_64 Linux runners, a CPU-bound benchmark can run under Valgrind for reproducible [instruction counts](#instruction-counts-with-callgrind) instead of noisy timings, with either setup.

## Pull requests from the same repository

One workflow runs on pushes to the default branch and on pull requests. Both upload from the benchmark process itself.

```yaml
name: Benchmark

on:
  push:
    branches: [main]
  pull_request:

jobs:
  benchmark:
    name: benchmark
    # Fork pull requests get no `id-token: write`, so there is nothing to upload.
    if: github.event_name == 'push' || github.event.pull_request.head.repo.full_name == github.repository
    runs-on: ubuntu-latest
    timeout-minutes: 20
    permissions:
      contents: read
      id-token: write # the upload authenticates with the job's OIDC token
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Run benchmark and upload report
        env:
          HOTPATH_UPLOAD: fail-on-error
          HOTPATH_UPLOAD_RESPONSE_PATH: ${{ github.workspace }}/upload.json
          HOTPATH_BENCHMARK: my_benchmark
          HOTPATH_USER_METADATA: runner=${{ runner.os }}-${{ runner.arch }},toolchain=stable,profile=release
          HOTPATH_OUTPUT_FORMAT: none
        run: |
          cargo run -q --release --example my_benchmark \
            --features hotpath-alloc,hotpath-cloud
          test -s "$HOTPATH_UPLOAD_RESPONSE_PATH"
```

Pushes to the default branch upload the **baseline** that pull requests are compared against. 

The response file is written only when the server accepted the report, and removed when the upload was skipped or failed, so `test -s` fails the job when no report was stored. Keep its path absolute: a relative one resolves against the working directory of the benchmark program. Give every uploading program its own file.

### Failing the job

`HOTPATH_UPLOAD` decides whether a failed upload fails the job:

| `HOTPATH_UPLOAD` | Upload fails |
|---|---|
| `enabled` (or `1`, `true`) | warning |
| `fail-on-error` | job fails |

A skipped upload (not in GitHub Actions, no OIDC token, invalid benchmark name, the program panicked) never fails the job. A regression or a broken budget never fails the job either: it is a warning in the job log. The pull request's `hotpath / <benchmark>` check fails when the policy sets [`pr_check = "fail"`](regression_policy.md#pull-request-check) and the finding comes from a block with `fail_check = true`: every budget rule by default, a policy section only when it opts in.

To block merging a pull request with a regression, make `hotpath / <benchmark>` a required check in the branch protection rules. A required check also blocks merges while hotpath.rs is unavailable, since no check is posted then.

### Environment variables

| Variable | Description |
|---|---|
| `HOTPATH_UPLOAD` | Upload the report when the run ends, and decide whether a failed upload fails the job: `enabled` or `fail-on-error` (see above). Unset, `0` or `false` uploads nothing. An unknown value uploads as `enabled` with a warning. (default: off) |
| `HOTPATH_BENCHMARK` | Name of the benchmark series, 1 to 64 characters from `[A-Za-z0-9._-]`. Reports are compared within one series, and every series gets a pull request comment of its own. An invalid name skips the upload. (default: `default`) |
| `HOTPATH_UPLOAD_RESPONSE_PATH` | File the server's response is written to as JSON when the report is accepted. A skipped or failed upload removes it. (default: not written) |
| `HOTPATH_POLICY_PATH` | Policy file to upload instead of `hotpath/<benchmark>-policy.toml` or `hotpath/policy.toml`. Relative to the working directory, and inside the repository. (default: not set) |
| `HOTPATH_USER_METADATA` | Comma-separated `key=value` pairs stored with the report and shown on its page, e.g. `runner=linux-x64,profile=release`. |
| `HOTPATH_SOURCE_ROOT` | Path of the build workspace relative to the repository root, for source links in the comment. Derived from the checkout when unset. |
| `HOTPATH_API_URL` | Base URL of the server. (default: `https://hotpath.rs`) |
| `HOTPATH_OUTPUT_FORMAT` | Set to `none` to keep the local report out of the job log. The upload does not depend on it. |

See [configuration](configuration.md) for the variables that shape the report itself.

## Pull requests from forks

A job triggered by a pull request from a fork runs the fork's code, so GitHub gives it a read-only token and no OIDC token, so the benchmark job cannot upload. It writes the report to an artifact instead, and a second workflow, triggered when the first one completes, uploads it.

### The benchmark workflow

Pushes to the default branch upload directly, as in the same-repository setup. Pull requests write the report to a file and store it as an artifact.

```yaml
name: Benchmark

on:
  push:
    branches: [main]
  pull_request:

jobs:
  upload:
    name: benchmark (push)
    if: github.event_name == 'push'
    runs-on: ubuntu-latest
    timeout-minutes: 20
    permissions:
      contents: read
      id-token: write
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Run benchmark and upload report
        env:
          HOTPATH_UPLOAD: fail-on-error
          HOTPATH_UPLOAD_RESPONSE_PATH: ${{ github.workspace }}/upload.json
          HOTPATH_BENCHMARK: my_benchmark
          HOTPATH_USER_METADATA: runner=${{ runner.os }}-${{ runner.arch }},toolchain=stable,profile=release
          HOTPATH_OUTPUT_FORMAT: none
        run: |
          cargo run -q --release --example my_benchmark \
            --features hotpath-alloc,hotpath-cloud
          test -s "$HOTPATH_UPLOAD_RESPONSE_PATH"

  report:
    name: benchmark (pull request)
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    timeout-minutes: 20
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@v4
        with:
          # The pull request's head commit, not the default merge commit.
          ref: ${{ github.event.pull_request.head.sha }}
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Run benchmark and write report
        env:
          HOTPATH_BENCHMARK: my_benchmark
          HOTPATH_USER_METADATA: runner=${{ runner.os }}-${{ runner.arch }},toolchain=stable,profile=release
          HOTPATH_OUTPUT_FORMAT: json
          HOTPATH_OUTPUT_PATH: report.json
        run: |
          cargo run -q --release --example my_benchmark \
            --features hotpath-alloc,hotpath-cloud
          test -s report.json
      - uses: actions/upload-artifact@v4
        with:
          name: hotpath-report
          path: report.json
          retention-days: 1
          if-no-files-found: error
```

The pull request job still builds with the `hotpath-cloud` feature. The feature is what adds the commit, the pull request, the benchmark name and the policy file to the JSON report. `HOTPATH_UPLOAD` is not set in that job: it has no token to upload with.

### The relay workflow

```yaml
name: Benchmark relay

on:
  workflow_run:
    workflows: ["Benchmark"] # the `name:` of the benchmark workflow
    types: [completed]

jobs:
  relay:
    uses: hotpath-rs/hotpath/.github/workflows/hotpath-relay.yml@{{HOTPATH_RELEASE_TAG}}
    permissions:
      actions: read # download the artifact of the benchmark run
      id-token: write # authenticate the upload
      pull-requests: read # find the pull request the run belongs to
    with:
      benchmark: my_benchmark
      artifact_name: hotpath-report
      report_file: report.json
```

[`hotpath-relay.yml`](https://github.com/hotpath-rs/hotpath/blob/main/.github/workflows/hotpath-relay.yml) is a reusable workflow. It runs after a successful benchmark run of a pull request, downloads the artifact, uploads the report and prints the server's response in the step summary. A rejected upload fails the relay job, and a comment that could not be posted is a warning. The pull request the report belongs to is looked up from the benchmark run's head commit, not read from the report, which the pull request's own code wrote; that lookup is what `pull-requests: read` is for.

Pin the relay to a release tag, the one of the `hotpath` version the benchmark builds with, so the relay and the report it uploads come from the same release. 

```bash
git ls-remote https://github.com/hotpath-rs/hotpath refs/tags/{{HOTPATH_RELEASE_TAG}}
```

Repositories that pin actions by commit hash should use the commit the tag points to, with the tag in the comment:

```yaml
    uses: hotpath-rs/hotpath/.github/workflows/hotpath-relay.yml@<commit sha> # {{HOTPATH_RELEASE_TAG}}
```

| Input | Description |
|---|---|
| `benchmark` | Name of the benchmark series, as in `HOTPATH_BENCHMARK`. Required. |
| `artifact_name` | Name of the artifact the benchmark job uploaded. (default: `hotpath-report`) |
| `report_file` | Name of the JSON report inside the artifact. (default: `report.json`) |
| `source_root` | Path of the build workspace relative to the repository root, `""` when they are the same. (default: `""`) |

Things to know about `workflow_run`:

- It runs the copy of the relay workflow that is on the **default branch**. Changes to it do nothing until they are merged.
- It matches the benchmark workflow by **name**. Rename that workflow and the relay stops firing, without an error.
- Its jobs are **not checks on the pull request**. The `hotpath / <benchmark>` check the server posts is, so a regression shows among the pull request's checks of fork pull requests too.

A pull request is judged under the policy file of its own branch, and that includes pull requests from forks. Review changes to `hotpath/*.toml` like changes to CI configuration.

### Environment variables

In the pull request job, which writes the report:

| Variable | Description |
|---|---|
| `HOTPATH_BENCHMARK` | Name of the benchmark series. The relay's `benchmark` input must name the same one. |
| `HOTPATH_OUTPUT_FORMAT` | Must be `json`. |
| `HOTPATH_OUTPUT_PATH` | File the report is written to, the one stored as the artifact. |
| `HOTPATH_POLICY_PATH` | Policy file to embed instead of the one found in `hotpath/`. |
| `HOTPATH_USER_METADATA` | Comma-separated `key=value` pairs stored with the report. |

The push job takes the variables of the [same-repository setup](#environment-variables).

## Instruction counts with Callgrind

Timing on shared GitHub runners is too noisy to judge small changes. A benchmark run under [Callgrind](callgrind.md) reports the instructions each function executed instead. Instruction counts measure computational work rather than speed, but they are reproducible for a deterministic workload, so a change of a few percent in a function's work stands out from run-to-run variation. Allocation profiling keeps working; timing is not reported under Valgrind.

It needs `hotpath` `0.29` or newer, an x86_64 Linux runner and a workload that finishes under Valgrind, which runs it tens of times slower. Add the feature next to the others:

```toml
[features]
hotpath = ["hotpath/hotpath"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
hotpath-cloud = ["hotpath", "hotpath/hotpath-cloud"]
hotpath-callgrind = ["hotpath", "hotpath/hotpath-callgrind"]
```

and judge the counts in the policy file under [`[functions.icount]`](regression_policy.md#functions-instructions).

The workflow is the [same-repository one](#pull-requests-from-the-same-repository) with the benchmark built first and its binary run under Valgrind:

```yaml
name: Benchmark

on:
  push:
    branches: [main]
  pull_request:

jobs:
  benchmark:
    name: benchmark
    # Fork pull requests get no `id-token: write`, so there is nothing to upload.
    if: github.event_name == 'push' || github.event.pull_request.head.repo.full_name == github.repository
    # Instruction counts only compare within one Valgrind build and Rust
    # toolchain, so the image, the package and the toolchain are pinned.
    runs-on: ubuntu-26.04
    timeout-minutes: 30
    permissions:
      contents: read
      id-token: write # the upload authenticates with the job's OIDC token
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: "1.99.0"
      - uses: Swatinem/rust-cache@v2
      - name: Install valgrind
        run: |
          sudo apt-get update
          sudo apt-get install -y valgrind=1:3.26.0-0ubuntu1
          valgrind --version
      - name: Build benchmark
        run: |
          cargo build -q --release --example my_benchmark \
            --features hotpath-alloc,hotpath-cloud,hotpath-callgrind
      - name: Run benchmark under Callgrind and upload report
        env:
          HOTPATH_UPLOAD: fail-on-error
          HOTPATH_UPLOAD_RESPONSE_PATH: ${{ github.workspace }}/upload.json
          HOTPATH_BENCHMARK: my_benchmark
          HOTPATH_USER_METADATA: runner=${{ runner.os }}-${{ runner.arch }},toolchain=1.99.0,profile=release,valgrind=3.26.0
          HOTPATH_OUTPUT_FORMAT: json
          HOTPATH_OUTPUT_PATH: ${{ runner.temp }}/report.json
          HOTPATH_CALLGRIND_OUT_FILE: ${{ runner.temp }}/callgrind.out.%p
        run: |
          valgrind -q --tool=callgrind --instr-atstart=no \
            --callgrind-out-file="$HOTPATH_CALLGRIND_OUT_FILE" \
            target/release/examples/my_benchmark
          test -s "$HOTPATH_UPLOAD_RESPONSE_PATH"
          jq -e '.functions_icount.data | length > 0' "$HOTPATH_OUTPUT_PATH" > /dev/null
```

What differs from the timing workflow:

- **Pinned environment.** Counts depend on the Valgrind version, the Rust toolchain and the OS, and hotpath.rs only compares reports whose Valgrind version, `rustc` and OS match: a mismatch is skipped, not judged. Pin the runner label, the Valgrind package and the toolchain instead of `ubuntu-latest` and `stable`, and bump them deliberately: the first push to the default branch after a bump uploads the new baseline. A runner label is not an immutable image, so system libraries can still change under it; the pins make reports comparable, not identical.
- **Build, then run the binary.** The workflow builds first and runs the compiled binary directly under Valgrind, which keeps the command and the out-file simple and predictable. Cargo's target runner (as in [Callgrind](callgrind.md#running-under-callgrind)) works too; `valgrind cargo run` does not, since it would instrument Cargo. Run from the repository root, so the git information and the policy file are found. For a binary target the path is `target/release/<name>`.
- **`--instr-atstart=no`** keeps Callgrind's instrumentation off until hotpath turns it on at guard creation. That cuts the cost of startup, but the program still runs under Valgrind the whole time.
- **`HOTPATH_CALLGRIND_OUT_FILE`** names the same file as `--callgrind-out-file`, here outside the checkout. Without it hotpath looks for `callgrind.out.<pid>` in the working directory.
- **The `jq` check.** When the Callgrind dump cannot be read, hotpath prints `[hotpath] icount report unavailable: ...` and the report carries allocations only; a run outside Valgrind reports timing instead. The check fails the job in both cases, so a run without instruction counts does not pass unnoticed. The JSON output is written next to the upload, which does not depend on it.
- **A longer timeout**, for Valgrind's slowdown.

Before relying on the verdicts, run the benchmark a few times on one commit (re-run the job) and set `min_percent_change` above the variation you see: how much concurrent code moves between runs depends on the workload.

For [pull requests from forks](#pull-requests-from-forks), make the same changes to both jobs of the benchmark workflow: the pinned runner, toolchain and Valgrind package, the `hotpath-callgrind` build, and the Valgrind run with `HOTPATH_CALLGRIND_OUT_FILE`. The push job also switches to `HOTPATH_OUTPUT_FORMAT: json` with a `HOTPATH_OUTPUT_PATH` for its `jq` check, as above. The pull request job keeps its read-only permissions and no `HOTPATH_UPLOAD`; it already writes the JSON report, so run the `jq` check on `report.json` before the artifact upload. The relay workflow does not change.

## Several benchmarks in one repository

Give every benchmark its own `HOTPATH_BENCHMARK` name and its own workflow or job. Each series has its own baseline and its own comment on the pull request. A series is judged under `hotpath/<benchmark>-policy.toml` when that file exists, and under the shared `hotpath/policy.toml` otherwise; without either, its uploads are refused. In the fork setup, every benchmark workflow also needs a unique `name:`, its own artifact name and its own relay workflow that names it in `workflows:`.

## Troubleshooting

| What you see | Cause |
|---|---|
| `upload skipped: not in GitHub Actions or missing ...` | The job has no OIDC token: `id-token: write` is missing, or the pull request comes from a fork. |
| `upload failed: the report carries no policy file` | The repository has no `hotpath/policy.toml` or `hotpath/<benchmark>-policy.toml`. Run `hotpath cloud init` and commit the file. |
| relay: `hotpath.rs rejected the report (HTTP 422): ... the report carries no policy file` | The pull request's branch has no policy file, for example a branch older than the commit that added it. Merge the default branch into it. |
| `upload failed: hotpath/policy.toml is not a valid policy: ...` | The policy file does not parse. `hotpath cloud validate-policy` shows the problem and its line. |
| `upload failed: the hotpath GitHub App is not installed on ... (HTTP 403)` | The GitHub App is not installed on the repository. Install it, or add the repository to the installation, from <a href="https://hotpath.rs/app" target="_blank" rel="noopener noreferrer">hotpath.rs/app</a>. |
| `upload failed: ... (HTTP 429)` | The repository is over its upload quota. |
| The comment reads "no baseline yet" | The base branch has no report in this series. It appears after the first push to the default branch with the workflow in place. |
| The comment reads "report not readable" | The report was produced by a `hotpath` version the server no longer reads. Update `hotpath`. |
| The report is uploaded but no comment or check appears | The upload line ends with `comment failed: ...`, and the job shows it as a warning. The usual cause is that the App's "Pull requests: write" or "Checks: write" permission is not approved for the installation. Approve it in the installation settings on GitHub (your account or organization settings, under Applications). |
| The relay does not run | The `workflows:` name does not match the benchmark workflow's `name:`, the relay workflow is not on the default branch yet, or the benchmark run failed. |
| `[hotpath] icount report unavailable: failed to read Callgrind dump ...` | Valgrind ran without `--tool=callgrind`, or `HOTPATH_CALLGRIND_OUT_FILE` differs from `--callgrind-out-file`. See [Instruction counts with Callgrind](#instruction-counts-with-callgrind). |
| The comment reads `functions_icount: environment mismatch (...)` | The two reports come from a different Valgrind version, `rustc` or OS, so their instruction counts were not compared. Pin all three; after a deliberate bump, the next push to the default branch uploads a comparable baseline. |
