# CI integration: benchmark Rust pull requests in GitHub Actions

`hotpath-cloud` reviews the performance of every pull request. Your CI job runs a benchmark with `hotpath` profiling enabled and uploads the report. hotpath.rs compares it with the report of the base branch and posts the result as a comment on the pull request.

What counts as a regression is customizable with a [regression policy](regression_policy.md) and [performance budgets](performance_budgets.md).

## Setup

Log in at <a href="https://hotpath.rs/app" target="_blank" rel="noopener noreferrer">hotpath.rs/app</a> and install the [hotpath-rs GitHub App](https://github.com/apps/hotpath-rs) on the repository from the dashboard. The app has no access to your code, only write access to pull requests to post comments, and to checks to report the result of each benchmark.

Add the `hotpath-cloud` feature next to the profiling features you use:

```toml
[dependencies]
hotpath = "{{HOTPATH_VERSION}}"

[features]
hotpath = ["hotpath/hotpath"]
hotpath-alloc = ["hotpath", "hotpath/hotpath-alloc"]
hotpath-cloud = ["hotpath", "hotpath/hotpath-cloud"]
hotpath-callgrind = ["hotpath", "hotpath/hotpath-callgrind"]
```

Add a policy file. Every benchmark needs one: its own `hotpath/<benchmark>-policy.toml`, or the shared `hotpath/policy.toml` when it has none. An upload without one is refused. `hotpath cloud init` writes the default policy to `hotpath/policy.toml`:

```bash
cargo install hotpath --features cloud
hotpath cloud init
```

Commit the file. The [regression policy](regression_policy.md) page explains what it contains and how to change it.

The benchmark is any program of yours instrumented with `hotpath`: an example, a binary or an integration test that runs a fixed workload. The report is uploaded at the end of the run.

There are two ways to wire the workflow, depending on where pull requests come from:

| Pull requests come from | Setup |
|---|---|
| branches of the same repository | [one workflow](#pull-requests-from-the-same-repository) that uploads from the benchmark job |
| forks (public repositories) | [two workflows](#pull-requests-from-forks): the benchmark job writes the report, a relay job uploads it |

The fork setup also handles pull requests from the same repository, so a project that accepts both needs only that one.

Both setups run the benchmark in **merged** mode (native timing + Callgrind instruction counts in one report), the recommended mode on x86_64 Linux runners. One build runs twice: natively, for timing, allocations and every other section, then under Valgrind's [Callgrind](callgrind.md), which adds instruction counts and uploads one merged report. Timing on shared runners is too noisy to judge small changes, while instruction counts are reproducible for a deterministic workload, so the counts are judged and the timing is shown next to them. Where Valgrind cannot run the benchmark, use [native mode](#native); [Profiling modes](#profiling-modes) compares the modes.

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
    # Instruction counts only compare within one Valgrind build and Rust
    # toolchain, so the image, the package and the toolchain are pinned.
    runs-on: ubuntu-26.04
    timeout-minutes: 30
    permissions:
      contents: read
      id-token: write # the upload authenticates with the job's OIDC token
    # Shared by both runs, so their reports describe one benchmark. Static
    # values: job-level `env` cannot read the `runner` context.
    env:
      HOTPATH_BENCHMARK: my_benchmark
      HOTPATH_USER_METADATA: runner=Linux-X64,toolchain=1.99.0,profile=release,valgrind=3.26.0
      HOTPATH_OUTPUT_FORMAT: json
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
      - name: Run benchmark natively
        env:
          HOTPATH_OUTPUT_PATH: ${{ runner.temp }}/native.json
        run: target/release/examples/my_benchmark
      - name: Run benchmark under Callgrind and upload the merged report
        env:
          HOTPATH_UPLOAD: fail-on-error
          HOTPATH_UPLOAD_RESPONSE_PATH: ${{ github.workspace }}/upload.json
          HOTPATH_OUTPUT_PATH: ${{ runner.temp }}/merged.json
          HOTPATH_MERGE_REPORT: ${{ runner.temp }}/native.json
          HOTPATH_CALLGRIND_OUT_FILE: ${{ runner.temp }}/callgrind.out.%p
        run: |
          valgrind -q --tool=callgrind --instr-atstart=no \
            --callgrind-out-file="$HOTPATH_CALLGRIND_OUT_FILE" \
            target/release/examples/my_benchmark
          test -s "$HOTPATH_UPLOAD_RESPONSE_PATH"
          jq -e '.meta.run == "merged" and (.functions_icount.data | length > 0) and (.functions_timing.data | length > 0)' \
            "$HOTPATH_OUTPUT_PATH" > /dev/null
```

Pushes to the default branch upload the **baseline** that pull requests are compared against.

What each part is for:

- **One build, two runs.** The benchmark is built once and the same binary runs natively, then under Valgrind. Never `valgrind cargo run`, which would instrument Cargo. For a binary target the path is `target/release/<name>`.
- **Shared settings in the job's `env`.** The benchmark name, the metadata and the JSON output must be the same for both runs, so they are set once for the job, with static values: job-level `env` cannot read the `runner` context, and the metadata describes the pinned environment anyway. `HOTPATH_UPLOAD`, `HOTPATH_MERGE_REPORT` and the output paths belong to their step. Set on the native step, `HOTPATH_UPLOAD` would upload a second, native-only report, and `HOTPATH_MERGE_REPORT` fails it, since that run is not under Valgrind.
- **The merge.** `HOTPATH_MERGE_REPORT` names the native run's report. The Callgrind run takes it as the base, adds its own instruction counts and uploads the result. It checks that both runs come from the same build, checkout, benchmark and CI run first. A failed merge fails the step whatever `HOTPATH_UPLOAD` says, writes no JSON and uploads nothing. [Timing and instruction counts in one report](callgrind.md#timing-and-instruction-counts-in-one-report) describes what a merged report carries.
- **Run from the repository root**, both runs from the same directory, so the git information and the policy file are found and the two reports describe the same checkout. The native report goes outside the checkout (`runner.temp`), to a different file than the merged one.
- **Pinned environment.** Instruction counts depend on the Valgrind version, the Rust toolchain and the OS, and hotpath.rs only compares counts whose Valgrind version, `rustc` and OS match: a mismatch is skipped, not judged. Pin the runner label, the Valgrind package and the toolchain instead of `ubuntu-latest` and `stable`, and bump them deliberately: the first push to the default branch after a bump uploads the new baseline. A runner label is not an immutable image, so system libraries can still change under it; the pins make reports comparable, not identical.
- **`--instr-atstart=no`** keeps Callgrind's instrumentation off until hotpath turns it on at guard creation. That cuts the cost of startup, but the program still runs under Valgrind the whole time.
- **`HOTPATH_CALLGRIND_OUT_FILE`** names the same file as `--callgrind-out-file`, here outside the checkout. Without it hotpath looks for `callgrind.out.<pid>` in the working directory.
- **`test -s` and the `jq` check.** The response file is written only when the server accepted the report, and removed when the upload was skipped or failed, so `test -s` fails the job when no report was stored. Keep its path absolute: a relative one resolves against the working directory of the benchmark program. Give every uploading program its own file. The `jq` check fails the job when the stored report is not a merged one with both instruction counts and timing.
- **A longer timeout**, for Valgrind, which runs the benchmark tens of times slower. The `hotpath-callgrind` feature also costs one extra call per measured invocation in the native run, which is visible in its timings.

The workload must run twice in one job doing the same work: fixed inputs and seeds, no network, and nothing the first run leaves behind (a database it fills, files it creates) changing the second run. Before relying on the verdicts, re-run the job a few times on one commit and set `min_percent_change` of [`[functions.icount]`](regression_policy.md#functions-instructions) above the variation you see: how much concurrent code moves between runs depends on the workload.

### Failing the job

`HOTPATH_UPLOAD` decides whether a failed upload fails the job:

| `HOTPATH_UPLOAD` | Upload fails |
|---|---|
| `enabled` (or `1`, `true`) | warning |
| `fail-on-error` | job fails |

A failed merge (`HOTPATH_MERGE_REPORT` set) always fails the job, whatever `HOTPATH_UPLOAD` says. A skipped upload (not in GitHub Actions, no OIDC token, invalid benchmark name, the program panicked) never fails the job. A regression or a broken budget never fails the job either: it is a warning in the job log. The pull request's `hotpath / <benchmark>` check fails when the policy sets [`pr_check = "fail"`](regression_policy.md#pull-request-check) and the finding comes from a block with `fail_check = true`: every budget rule by default, a policy section only when it opts in.

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
| `HOTPATH_OUTPUT_FORMAT` | `json` in merged mode, for both runs: the native run writes the report the merge reads, the Callgrind run the merged report the `jq` check reads. In native mode, `none` keeps the local report out of the job log. The upload does not depend on it. |
| `HOTPATH_OUTPUT_PATH` | File the JSON report is written to. Set per step in merged mode, each run to its own file. |
| `HOTPATH_MERGE_REPORT` | On the Callgrind step only: the native run's JSON report, merged into this run's report before it is written and uploaded. A failed merge fails the job whatever `HOTPATH_UPLOAD` says. (default: not set) |
| `HOTPATH_CALLGRIND_OUT_FILE` | The `--callgrind-out-file` value Valgrind was started with; `%p` and `%q{VAR}` are expanded like Valgrind does. (default: `callgrind.out.%p`) |

See [configuration](configuration.md) for the variables that shape the report itself.

## Pull requests from forks

A job triggered by a pull request from a fork runs the fork's code, so GitHub gives it a read-only token and no OIDC token, so the benchmark job cannot upload. It writes the report to an artifact instead, and a second workflow, triggered when the first one completes, uploads it.

### The benchmark workflow

Pushes to the default branch upload directly, as in the same-repository setup. Pull requests run the same two runs, write the merged report to a file and store it as an artifact.

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
    runs-on: ubuntu-26.04
    timeout-minutes: 30
    permissions:
      contents: read
      id-token: write
    env:
      HOTPATH_BENCHMARK: my_benchmark
      HOTPATH_USER_METADATA: runner=Linux-X64,toolchain=1.99.0,profile=release,valgrind=3.26.0
      HOTPATH_OUTPUT_FORMAT: json
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
      - name: Build benchmark
        run: |
          cargo build -q --release --example my_benchmark \
            --features hotpath-alloc,hotpath-cloud,hotpath-callgrind
      - name: Run benchmark natively
        env:
          HOTPATH_OUTPUT_PATH: ${{ runner.temp }}/native.json
        run: target/release/examples/my_benchmark
      - name: Run benchmark under Callgrind and upload the merged report
        env:
          HOTPATH_UPLOAD: fail-on-error
          HOTPATH_UPLOAD_RESPONSE_PATH: ${{ github.workspace }}/upload.json
          HOTPATH_OUTPUT_PATH: ${{ runner.temp }}/merged.json
          HOTPATH_MERGE_REPORT: ${{ runner.temp }}/native.json
          HOTPATH_CALLGRIND_OUT_FILE: ${{ runner.temp }}/callgrind.out.%p
        run: |
          valgrind -q --tool=callgrind --instr-atstart=no \
            --callgrind-out-file="$HOTPATH_CALLGRIND_OUT_FILE" \
            target/release/examples/my_benchmark
          test -s "$HOTPATH_UPLOAD_RESPONSE_PATH"
          jq -e '.meta.run == "merged" and (.functions_icount.data | length > 0) and (.functions_timing.data | length > 0)' \
            "$HOTPATH_OUTPUT_PATH" > /dev/null

  report:
    name: benchmark (pull request)
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-26.04
    timeout-minutes: 30
    permissions:
      contents: read
    env:
      HOTPATH_BENCHMARK: my_benchmark
      HOTPATH_USER_METADATA: runner=Linux-X64,toolchain=1.99.0,profile=release,valgrind=3.26.0
      HOTPATH_OUTPUT_FORMAT: json
    steps:
      - uses: actions/checkout@v4
        with:
          # The pull request's head commit, not the default merge commit.
          ref: ${{ github.event.pull_request.head.sha }}
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: "1.99.0"
      - uses: Swatinem/rust-cache@v2
      - name: Install valgrind
        run: |
          sudo apt-get update
          sudo apt-get install -y valgrind=1:3.26.0-0ubuntu1
      - name: Build benchmark
        run: |
          cargo build -q --release --example my_benchmark \
            --features hotpath-alloc,hotpath-cloud,hotpath-callgrind
      - name: Run benchmark natively
        env:
          HOTPATH_OUTPUT_PATH: ${{ runner.temp }}/native.json
        run: target/release/examples/my_benchmark
      - name: Run benchmark under Callgrind and write the merged report
        env:
          HOTPATH_OUTPUT_PATH: report.json
          HOTPATH_MERGE_REPORT: ${{ runner.temp }}/native.json
          HOTPATH_CALLGRIND_OUT_FILE: ${{ runner.temp }}/callgrind.out.%p
        run: |
          valgrind -q --tool=callgrind --instr-atstart=no \
            --callgrind-out-file="$HOTPATH_CALLGRIND_OUT_FILE" \
            target/release/examples/my_benchmark
          test -s report.json
          jq -e '.meta.run == "merged" and (.functions_icount.data | length > 0) and (.functions_timing.data | length > 0)' \
            report.json > /dev/null
      - uses: actions/upload-artifact@v4
        with:
          name: hotpath-report
          path: report.json
          retention-days: 1
          if-no-files-found: error
```

The pull request job still builds with the `hotpath-cloud` feature. The feature is what adds the commit, the pull request, the benchmark name and the policy file to the JSON report. `HOTPATH_UPLOAD` is not set in that job: it has no token to upload with. A failed merge fails it before the artifact upload, so the relay never posts a report that is not merged. The parts the two jobs share are explained in the [same-repository setup](#pull-requests-from-the-same-repository).

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
| `HOTPATH_OUTPUT_PATH` | File the report is written to: the native report for the native step, the merged report stored as the artifact for the Callgrind step. |
| `HOTPATH_MERGE_REPORT` | On the Callgrind step only: the native run's JSON report. |
| `HOTPATH_CALLGRIND_OUT_FILE` | The `--callgrind-out-file` value Valgrind was started with. |
| `HOTPATH_POLICY_PATH` | Policy file to embed instead of the one found in `hotpath/`. |
| `HOTPATH_USER_METADATA` | Comma-separated `key=value` pairs stored with the report. |

The push job takes the variables of the [same-repository setup](#environment-variables).

<a id="instruction-counts-with-callgrind"></a>

## Profiling modes

| Mode | Report carries | Judged by default | Use when |
|---|---|---|---|
| merged (recommended) | native timing, allocations and every other native section, plus instruction counts | instruction counts, allocations; timing shown, not judged | x86_64 Linux runner, a CPU-bound and mostly deterministic workload that runs twice the same way and finishes under Valgrind |
| native | timing, allocations and every other section | allocations; timing shown, not judged | no Valgrind (macOS, Windows or ARM runners), a workload too slow under Valgrind, or one that cannot run twice the same way |
| Callgrind only | instruction counts and allocations, no timing | instruction counts, allocations | existing setups; not recommended for new benchmarks |

Merged mode needs:

- `hotpath` `0.29.2` or newer;
- an x86_64 Linux runner;
- a CPU-bound workload: its time goes into computation, not into network, disk, sleeps, timers or a database service, and it is not dominated by many threads racing each other (Valgrind serializes threads, so their interleaving moves the counts);
- the code to judge in functions with `#[hotpath::measure]` (or `measure_all`): `measure_block!` blocks are not counted;
- a `[profile.release]` that keeps symbols (no `strip = true` or `"symbols"`);
- a workload that finishes under Valgrind within the job timeout: one that takes more than about a minute natively should be made smaller or run natively;
- a workload that runs twice in one job doing the same work.

### Merged

The workflows of both setups above. The judged instruction counts are configured under [`[functions.icount]`](regression_policy.md#functions-instructions), and timing stays listed under `[functions.timing]` with `judged = false`, as the default policy writes it, so it shows next to the counts.

### Native

Without Valgrind, the benchmark runs once and uploads its native report:

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
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: "1.99.0"
      - uses: Swatinem/rust-cache@v2
      - name: Run benchmark and upload report
        env:
          HOTPATH_UPLOAD: fail-on-error
          HOTPATH_UPLOAD_RESPONSE_PATH: ${{ github.workspace }}/upload.json
          HOTPATH_BENCHMARK: my_benchmark
          HOTPATH_USER_METADATA: runner=${{ runner.os }}-${{ runner.arch }},toolchain=1.99.0,profile=release
          HOTPATH_OUTPUT_FORMAT: none
        run: |
          cargo run -q --release --example my_benchmark \
            --features hotpath-alloc,hotpath-cloud
          test -s "$HOTPATH_UPLOAD_RESPONSE_PATH"
```

The build needs no `hotpath-callgrind` feature. In the fork setup, both jobs take the same shape: no Valgrind, no native step and no merge, one `cargo run`; the pull request job keeps writing `report.json` and storing it as the artifact.

### Callgrind only

A benchmark that already runs only under Callgrind keeps working: its report carries instruction counts and allocations, and no timing, since durations under Valgrind are meaningless. It is the merged workflow without the native step and without `HOTPATH_MERGE_REPORT`, with the `jq` check reduced to `.functions_icount.data | length > 0`. New benchmarks should use merged mode, which adds timing at the cost of one native run.

## Several benchmarks in one repository

Give every benchmark its own `HOTPATH_BENCHMARK` name and its own workflow or job. Each series has its own baseline and its own comment on the pull request. A series is judged under `hotpath/<benchmark>-policy.toml` when that file exists, and under the shared `hotpath/policy.toml` otherwise; without either, its uploads are refused. In the fork setup, every benchmark workflow also needs a unique `name:`, and one relay workflow serves all of them. List every benchmark workflow in `workflows:` and look up the series from the name of the workflow that triggered the run. Artifacts belong to one run, so every pull request job keeps the default `hotpath-report` artifact with `report.json` in it:

```yaml
name: Benchmark relay

on:
  workflow_run:
    workflows: ["Benchmark", "Parser benchmark"]
    types: [completed]

jobs:
  relay:
    uses: hotpath-rs/hotpath/.github/workflows/hotpath-relay.yml@{{HOTPATH_RELEASE_TAG}}
    permissions:
      actions: read
      id-token: write
      pull-requests: read
    with:
      # benchmark workflow `name:` -> its `HOTPATH_BENCHMARK`
      benchmark: ${{ fromJSON('{"Benchmark":"my_benchmark","Parser benchmark":"parser"}')[github.event.workflow_run.name] }}
```

Keep `workflows:` and the lookup in sync: a workflow missing from `workflows:` is never relayed, and one missing from the lookup is relayed with an empty `benchmark`, which fails the upload.

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
| `[hotpath] icount report unavailable: failed to read Callgrind dump ...`, or `merge failed: no instruction counts: failed to read Callgrind dump ...` | Valgrind ran without `--tool=callgrind`, or `HOTPATH_CALLGRIND_OUT_FILE` differs from `--callgrind-out-file`. See [Profiling modes](#profiling-modes). |
| `merge failed: the native report's ... is ..., this run's is ...` | The two runs differ in build, checkout, benchmark or CI run. Run both from one build, in one job, from the same directory. |
| `merge failed: this run is not under valgrind --tool=callgrind` | `HOTPATH_MERGE_REPORT` is set on the native step, or for the whole job. Set it only on the Callgrind step. |
| `merge failed: cannot read the native report ...` | The path is wrong, or the native step did not run first or did not write JSON (`HOTPATH_OUTPUT_FORMAT: json`). |
| `merge failed: the native report has no timing to merge ...` | The native run had function timing off: a sampling rate of `0`, or timing left out of `HOTPATH_REPORT`. |
| `upload failed: the report is ... bytes, over the 5 MiB ... limit of hotpath.rs` | The report is too large to upload, and is never cut to fit. Measure fewer functions or split the benchmark. The relay reports the same limit as an `::error::`. |
| The `jq` check fails although the upload succeeded | The stored report is not merged: `HOTPATH_MERGE_REPORT` is missing on the Callgrind step, or the native run wrote no timing. |
| The comment reads `functions_icount: environment mismatch (...)` | The two reports come from a different Valgrind version, `rustc` or OS, so their instruction counts were not compared. Pin all three; after a deliberate bump, the next push to the default branch uploads a comparable baseline. |
