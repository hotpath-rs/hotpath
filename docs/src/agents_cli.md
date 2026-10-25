# Agents CLI: let AI coding agents fix Rust performance regressions

`hotpath cloud` is the command line client of the hotpath-cloud API. It gives a coding agent, a script or a CI step the same judgement the pull request comment shows, as JSON: which report was compared with which baseline, under which policy, what regressed, which budgets broke, and the verdict. An agent can push a change, wait for the benchmark, read the verdict and fix what regressed, without opening the dashboard.

Give your agent the `hotpath-rs` [agent skill](https://github.com/hotpath-rs/hotpath/blob/main/skills/hotpath-rs/SKILL.md) (`npx skills add hotpath-rs/hotpath --skill hotpath-rs`): it points the agent to the [step-by-step workflow](https://hotpath.rs/agent/cloud_cli) for these commands, waiting for a benchmark, reading the verdict and fixing what regressed.

## Install and authenticate

```bash
cargo install hotpath --features cloud
```

Create a personal API token at <a href="https://hotpath.rs/app/tokens" target="_blank" rel="noopener noreferrer">hotpath.rs/app/tokens</a> and export it:

```bash
export HOTPATH_API_TOKEN=...
hotpath cloud auth --pretty
```

```json
{
  "login": "pawurb",
  "token": {
    "name": "laptop",
    "expires_at": "2026-12-25T11:27:12.311073Z"
  }
}
```

The token gives the CLI access to the same repositories as your account on hotpath.rs: the ones your GitHub user can see that have the hotpath GitHub App installed. They are listed on <a href="https://hotpath.rs/app" target="_blank" rel="noopener noreferrer">hotpath.rs/app</a>, where you can also add more.

## CLI commands

### repos and benchmarks

`repos` lists every repository the token reaches, with its benchmarks. `benchmarks` lists one repository's:

```bash
hotpath cloud benchmarks --repo hotpath-rs/hotpath --pretty
```

```json
{
  "repository": "hotpath-rs/hotpath",
  "benchmarks": [
    { "name": "drain", "reports": 55, "latest_report_at": "2026-10-01T23:09:00.46559Z" },
    { "name": "meta", "reports": 54, "latest_report_at": "2026-10-01T23:09:29.367532Z" }
  ]
}
```

### report

Fetches one stored report, selected by pull request, commit or id:

```bash
hotpath cloud report --repo hotpath-rs/hotpath --benchmark drain --pr 604 --no-payload --pretty
```

```json
{
  "id": "01a0f476-5b3a-7d0e-901a-7756f28e2414",
  "event": "pull_request",
  "commit_sha": "ac5d8dcb4d21b714e0d2a04fd837637b3e1d1356",
  "head_ref": "perf/cycle-drain-buffer",
  "pr_number": 604,
  "baseline_id": "01a0f43f-c0f8-71c0-bfcc-ef82bbb17270",
  "comment_url": "https://github.com/hotpath-rs/hotpath/pull/604#issuecomment-5783640286",
  "regressed": false,
  "policy_path": "hotpath/drain-policy.toml",
  "dashboard_url": "https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/drain/reports/01a0f476-5b3a-7d0e-901a-7756f28e2414"
}
```

| Flag | Selects |
|---|---|
| `--pr N` | the newest report of the pull request |
| `--commit SHA` | the newest report of a full 40-character commit sha, measured or as the pull request's head |
| `--id ID` | the report with this id |
| `--event push\|pull_request` | narrows `--pr` or `--commit` to one event |

`--commit $(git rev-parse HEAD)` works on a pull request branch: CI measures GitHub's merge commit, and the report also matches the pull request's head commit. Without `--no-payload` the answer also carries the full uploaded hotpath report.

A report that is not there yet answers `not_found` with exit code 1, so polling `report` until it exits 0 waits for CI's upload.

### diff

Compares a report with its baseline under the policy the report carried, exactly as the pull request comment did. It selects the report with the same flags as `report`:

```bash
hotpath cloud diff --repo hotpath-rs/hotpath --benchmark drain --pr 604 --pretty
```

Trimmed answer:

```json
{
  "rows": "findings",
  "verdict": {
    "judged": true,
    "regressed": false,
    "regressions": 0,
    "improvements": 1,
    "budgets_broken": 0
  },
  "budgets": { "rules": 1, "broken": 0, "findings": [], "notes": [] },
  "result": {
    "status": "compared",
    "sections": [
      {
        "resource": "functions",
        "kind": "alloc",
        "family": { "name": "alloc", "judged": true, "min_percent_change": 2.0, "metrics": ["avg", "total"] },
        "rows": [
          {
            "name": "hotpath_drain::EventProducer::push",
            "location": { "file": "crates/hotpath-drain/src/lib.rs", "line": 194 },
            "outcome": "improvement",
            "cells": [
              { "column": "avg", "unit": "bytes", "base": "24 B", "head": "1 B", "change_percent": -95.83, "crossed": "down" },
              { "column": "total", "unit": "bytes", "base": "23.1 MB", "head": "1.2 MB", "change_percent": -94.98, "crossed": "down" }
            ]
          }
        ]
      }
    ]
  },
  "dashboard_url": "https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/drain/reports/01a0f476-5b3a-7d0e-901a-7756f28e2414/diff"
}
```

How to read it:

- `verdict` covers the comparison and the [budgets](performance_budgets.md) together. `judged` says whether anything was judged, `regressed` whether a judged section regressed or a budget broke.
- `result.status` is `compared`, `no_baseline` or `unreadable`. Budgets are checked without a baseline too.
- Each section is one section of the [regression policy](regression_policy.md). A row's `outcome` is `regression`, `improvement`, `added` or `removed`, and its `cells` are the values that explain it. A cell with `crossed` went past the policy's `min_percent_change`, `up` or `down`.
- Values are strings formatted as the comment shows them, with their `unit`. `change_percent` is the exact number to reason about.
- `budgets.findings` lists the broken budget checks, each with its entry, `actual`, `limit` and the rule's `message`.

By default the answer lists only what the comment lists. `--advisory` adds the findings of sections the policy lists with `judged = false`, which never count toward the verdict. `--full` lists every row and every value.

`diff` exits `0` only when the report was judged and nothing regressed. A regression, a broken budget, or nothing to judge (no baseline and no budgets) exits `1` with the answer on stdout.

### validate-policy and init

`init` writes the default policy to `hotpath/policy.toml`, and `validate-policy` checks a policy file before you push it. See [regression policy](regression_policy.md#policy-file).

```bash
hotpath cloud init
hotpath cloud validate-policy --pretty
```

`init --benchmark NAME` writes `hotpath/NAME-policy.toml`, the policy of that benchmark alone, and `validate-policy --benchmark NAME` checks the file a run of that benchmark picks. `init` refuses to replace an existing file unless you pass `--force`.

## Output and exit codes

Every command prints one JSON document on stdout, compact by default, indented with `--pretty`, or written to a file with `--output FILE`. An error prints one JSON document on stderr and nothing on stdout:

```json
{"error":"No report for PR #999999 in benchmark drain.","code":"not_found"}
```

| Exit code | Meaning |
|---|---|
| `0` | success; for `diff`, the report was judged and nothing regressed |
| `1` | an error (not found, auth, network, invalid argument value), or for `diff` any answer that is not "judged and fine" |
| `2` | usage error |
