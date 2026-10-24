# Regression policy: decide what counts as a performance regression

The regression policy is a TOML file in your repository, `hotpath/policy.toml`, that decides which changes between a pull request and its baseline count as regressions. For every profiled resource it sets how large a change must be to matter, which metrics decide, and which entries to ignore. To set absolute limits that apply regardless of the baseline, add [performance budgets](performance_budgets.md) to the same file.

## Policy file

### Creating one

`hotpath cloud init` writes a starting policy to `hotpath/policy.toml`:

```bash
cargo install hotpath --features cloud
hotpath cloud init
```

`--benchmark NAME` writes `hotpath/NAME-policy.toml`, a policy for that benchmark only. An existing file is never replaced unless you pass `--force`.

The starting file turns on the [pull request check](#pull-request-check) and lists the three [function sections](#functions-timing). To compare SQL queries, routes, locks or any other resource, add its section from [Sections](#sections): a section the file leaves out is not compared.

### Which file a run uses

A run picks its policy file in this order:

1. the file `HOTPATH_POLICY_PATH` names, relative to the working directory or absolute. When the variable is set, the two files below are never tried
2. `hotpath/<benchmark>-policy.toml`, a benchmark's own policy
3. `hotpath/policy.toml`, shared by every benchmark without its own

An upload without a policy file is refused. So is one whose picked file cannot be sent: a `HOTPATH_POLICY_PATH` that names a missing file, or a file that is unreadable, blank, not valid UTF-8, larger than 65536 bytes or outside the repository. The lookup does not fall through to the next file in that case. A pull request is judged under the policy of its own branch, so review changes to `hotpath/*.toml` like changes to CI configuration.

### Validating

Check a file before you push with `hotpath cloud validate-policy`. A refused file lists its problems with their line:

```json
{
  "path": "hotpath/policy.toml",
  "valid": false,
  "problems": [
    {
      "line": 3,
      "message": "unknown field `min_chnage`, expected one of `judged`, `fail_check`, `min_percent_change`, `metrics`, `min_calls`"
    }
  ]
}
```

An upload with an invalid policy is refused, and the pull request comment says why.

## Sections

A section is one table per resource and metric family, such as `[functions.alloc]`. Each block below shows the built-in values, which a key you leave out takes; the `ignore` patterns are examples. **A section you leave out is hidden everywhere**: it is not compared, not shown on the comparison page and not in the comment.

### Functions timing

Execution time of [instrumented functions](functions.md). Timing on a shared CI runner varies from run to run, so it is listed without being judged.

```toml
[functions.timing]
judged = false
min_percent_change = 50
metrics = ["avg", "total"]
min_calls = 1
```

Metrics: `avg` and `total` time per function, percentiles such as `p95`.

### Functions instructions

Instructions executed by instrumented functions, from benchmarks run under [Callgrind](callgrind.md) with the `hotpath-callgrind` feature (the report's `functions_icount` section). Instruction counts measure computational work, not elapsed time; a deterministic workload executes nearly the same instructions on every run, so a small change usually reflects a change in the code's work.

```toml
[functions.icount]
judged = true
fail_check = false
min_percent_change = 2
metrics = ["avg", "total"]
min_calls = 1
```

Metrics: `avg` and `total` instructions per function. Callgrind counts have no percentiles.

Counts only compare within one environment: when the two reports differ in Valgrind version, `rustc` or OS, or one side lacks the Valgrind version, the section is skipped as an environment mismatch and not judged. Pin them in CI, see [CI integration](ci_integration.md#instruction-counts-with-callgrind).

The built-in 2% suits deterministic single-threaded workloads. Thread interleaving moves the counts of concurrent code between runs of one commit by an amount that depends on the workload, so run the benchmark a few times, measure that spread and set `min_percent_change` above it; 3% is a conservative start.

### Functions allocations

Memory allocated by instrumented functions, from builds with the `hotpath-alloc` feature. A fixed workload allocates the same amount on every run, so even small changes are real.

```toml
[functions]
ignore = ["my_crate::test_support::*"]
min_percent_total = 2

[functions.alloc]
judged = true
min_percent_change = 20
metrics = ["avg", "total"]
min_calls = 1
```

Metrics: `avg` and `total` allocated per function, percentiles such as `p95`. `ignore` matches the function name, and the `[functions]` keys apply to all three function sections.

### SQL queries

Execution time of [SQL queries](sql_tracing.md), one row per query, route and source function.

```toml
[sql]
ignore = ["CREATE TABLE*"]
min_percent_total = 0.5

[sql.timing]
judged = false
min_percent_change = 50
metrics = ["avg", "total"]
min_calls = 0
```

Metrics: `avg`, `total`, percentiles. `ignore` matches `query | route | source`, so `SELECT * FROM sessions*` matches the query wherever it ran.

### HTTP requests

Duration and errors of [outbound HTTP requests](http_tracing.md), one row per endpoint, route and source function.

```toml
[http.timing]
judged = false
min_percent_change = 50
metrics = ["avg", "total"]
min_calls = 0
```

Metrics: `avg`, `total`, percentiles, `errors`.

### Server timing

Response time, errors and downstream calls of each [served route](axum_tracing.md).

```toml
[server]
ignore = ["GET /health"]
min_percent_total = 0.5

[server.timing]
judged = true
min_percent_change = 50
metrics = ["avg", "total"]
min_calls = 0
```

Metrics: `avg`, `total`, percentiles, `status_4xx`, `status_5xx`, `sql_per_request`, `http_per_request`. `ignore` matches the route, e.g. `GET /users/{id}`.

### Server allocations

Memory allocated while serving each route, from builds with the `hotpath-alloc` feature.

```toml
[server.alloc]
judged = true
min_percent_change = 2
metrics = ["avg", "total"]
min_calls = 0
```

Metrics: `avg`, `total`, percentiles, `bytes_per_request`, `allocs_per_request`.

### Mutexes

Contention on [mutexes](locks.md): how long callers wait for the lock and how long it is held.

```toml
[mutexes.timing]
judged = false
min_percent_change = 50
metrics = ["wait_avg", "acquire_avg"]
min_calls = 0
```

Metrics: `wait_avg`, `acquire_avg`, percentiles such as `wait_p95`. `ignore` matches the lock's label.

### RwLocks

Contention on [read-write locks](locks.md), with reads and writes measured apart.

```toml
[rw_locks.timing]
judged = false
min_percent_change = 50
metrics = ["read_wait_avg", "write_wait_avg", "read_acquire_avg", "write_acquire_avg"]
min_calls = 0
```

Metrics: the four averages above, percentiles such as `read_wait_p95`. `ignore` matches the lock's label.

### Channels

Delay, backlog and throughput of [channels](data_flow.md).

```toml
[channels.flow]
judged = false
min_percent_change = 50
metrics = ["delay_avg", "max_queue_size"]
min_calls = 0
```

Metrics: `delay_avg`, percentiles such as `delay_p95`, `max_queue_size`, `sent_per_sec`, `received_per_sec`. For the two rates a drop is the regression. `ignore` matches the channel's label.

### I/O

Latency, throughput and errors of [I/O streams](io_tracing.md), one row per stream and operation.

```toml
[io.flow]
judged = false
min_percent_change = 50
metrics = ["avg", "errors", "bytes_per_sec"]
min_calls = 0
```

Metrics: `avg`, `total`, percentiles, `errors`, `bytes_per_sec` (a drop is the regression). `ignore` matches `label | operation | -`, so `stdout*` matches every operation on it.

### Threads

Memory allocated by named [threads](threads.md), from builds with the `hotpath-alloc` feature.

```toml
[threads]
names = ["db-writer", "rt-worker"]

[threads.alloc]
judged = false
min_percent_change = 20
metrics = ["total"]
min_calls = 0
```

Metrics: `total`. Threads sharing a name, such as a pool's workers, are summed into one row; `min_calls` reads how many threads carry the name.

**Only threads listed in `names` are compared**, matched exactly. An unnamed thread shows up as `thread_<id>` with an id that changes every run, so name the ones you want compared:

```rust
std::thread::Builder::new()
    .name("db-writer".into())
    .spawn(move || run_writer(rx))?;
```

On Linux a thread name is cut to 15 bytes, so `tokio-runtime-worker` shows up as `tokio-runtime-w`. List the name as the report shows it.

## Options

### Section keys

Every section takes the same five keys:

| Key | Meaning |
|---|---|
| `judged` | Whether the section decides the verdict and reaches the pull request comment. `false` keeps it on the comparison page as context. |
| `fail_check` | Whether a regression of this section fails the [pull request check](#pull-request-check). `false` by default: a relative change can be noise, so it is reported without failing anything. Needs `judged = true`. |
| `min_percent_change` | The smallest change, in percent and in either direction, that counts, from `0` to `1000`. |
| `metrics` | The metrics that are judged. A row regresses when any of them crosses `min_percent_change` in its worse direction. |
| `min_calls` | Rows with fewer calls than this on both sides are not judged. |

Percentiles are written the way hotpath names them (`p95`, `p99.9`), and a report has only the ones it was profiled with (`p95` by default, set with `percentiles` on [`#[hotpath::main]`](functions.md)).

### Resource keys

The resource tables (`[functions]`, `[sql]`, `[http]`, `[server]`, `[mutexes]`, `[rw_locks]`, `[channels]`, `[io]`) take:

| Key | Meaning |
|---|---|
| `ignore` | Patterns of entries that are never judged. `*` matches any run of characters, and a pattern must match the whole name. |
| `min_percent_total` | Rows below this share of the run, in percent, are skipped as noise. Only `functions`, `sql`, `http` and `server` have it. |

`[threads]` takes `names` instead of `ignore`, see [Threads](#threads).

## Pull request feedback

### Pull request comment

By default every pull request upload posts or updates a comment with its result. To comment only when something changed, set `pr_comment` at the top of the file, before any table:

```toml
pr_comment = "on_change"

[functions]
min_percent_total = 2
```

| Value | When the comment is posted |
|---|---|
| `"always"` | On every pull request upload. The default. |
| `"on_change"` | Only when the upload finds a regression, an improvement or a broken [budget](performance_budgets.md). |

With `"on_change"`, a comment the pull request already has is still updated, so an earlier regression never stays up after a fix. An upload the server refuses, or a report it cannot read, always gets a comment explaining why.

### Pull request check

A pull request upload can also post a `hotpath / <benchmark>` check on the pull request's head commit. Set `pr_check` at the top of the file to choose when; the file `hotpath cloud init` writes sets `"fail"`:

```toml
pr_check = "fail"

[functions]
min_percent_total = 2

[functions.alloc]
judged = true
fail_check = true
```

`pr_check` decides whether the check can fail; `fail_check` on each block decides what fails it. **The check fails only when `pr_check` is `"fail"` or `"failures_only"` and a finding comes from a block with `fail_check = true`.** A broken [budget](performance_budgets.md) rule has `fail_check = true` unless it says otherwise; a section has `fail_check = false` unless it opts in, like `[functions.alloc]` above.

| `pr_check` | Check |
|---|---|
| `"off"` | None, the comment is the whole report. The value when the key is left out. |
| `"report"` | A dry run that never fails: findings that would fail the check are marked "would fail the check" in the comment, and the check is neutral. |
| `"fail"` | Fails on a finding from a `fail_check` block. Any other regression or broken budget is neutral. |
| `"failures_only"` | Like `"fail"`, but posted only when it fails, nothing otherwise. The least noise. |

Where a check is posted, a clean result is a success, and nothing judged (no baseline yet and no budgets) is neutral. The comment marks each finding that fails the check. A `"failures_only"` check cannot be made required: a clean pull request never gets one, so GitHub would wait for it. A clean re-run on the same commit also leaves an earlier failure in place; a new push clears it.

An upload the server refuses gets no check. A re-run posts a new check, and GitHub shows the latest. Put the key in `hotpath/<benchmark>-policy.toml` to use the check for one benchmark only. To block merging, use `"fail"` and make the check required, see [CI integration](ci_integration.md#failing-the-job).
