# Read hotpath Cloud verdicts and fix performance regressions

Instructions for an AI coding agent. `hotpath cloud` is the command line client of hotpath.rs. CI uploads a hotpath benchmark report on every push to the default branch and every pull request. hotpath.rs compares each pull request report with its baseline (the report of the base branch), judges it under the repository's policy file and posts a comment on the pull request. The CLI returns the same judgement as JSON.

## Documentation

The hotpath docs are the reference for every command, flag, JSON field and policy key; this page only holds the workflow. Every page of https://hotpath.rs is served as markdown when requested with an `Accept: text/markdown` header:

```bash
curl -fsSL -H 'Accept: text/markdown' https://hotpath.rs/agents_cli
```

Fetch https://hotpath.rs/agents_cli before the first `hotpath cloud` command, and the other pages when the task reaches them, instead of relying on memory. Keep `-fsSL` and the header. If a fetch fails, stop and tell the user which URL could not be retrieved; do not continue from memory. A `#fragment` is not sent to the server: fetch the whole page and find the heading.

The site documents the latest hotpath release, which can be newer than the version the project uses. Never change the project's `hotpath` version because the docs show a newer one. If the docs describe something that version does not have, tell the user it needs a newer hotpath instead of working around it.

| Page | Covers |
|---|---|
| https://hotpath.rs/agents_cli | install and authenticate, `repos`, `benchmarks`, `report`, `diff` (flags, JSON answer, how to read it), `validate-policy`, `init`, output and exit codes |
| https://hotpath.rs/regression_policy | the policy file: which file a run uses, sections, keys, the pull request comment and check |
| https://hotpath.rs/performance_budgets | budget rules, units, broken budgets |
| https://hotpath.rs/ci_integration | the CI workflow that uploads reports, "Profiling modes" (merged, native, Callgrind only), troubleshooting uploads |
| https://hotpath.rs/callgrind | instruction counts (`icount`): merged reports, what is counted, reproducibility limits, comparing runs |

## Setup

1. Check the CLI: `hotpath cloud --help`. If it is missing or has no `cloud` command, install it as "Install and authenticate" describes.
2. Check the token: `hotpath cloud auth`. It reads `HOTPATH_API_TOKEN`. If it fails with `invalid_token` or the variable is unset, ask the user to create a token at https://hotpath.rs/app/tokens and export it. Never ask the user to paste the token into the conversation.
3. Find the repository (`OWNER/NAME`, from `git remote get-url origin`) and the benchmark name: `hotpath cloud benchmarks --repo OWNER/NAME`. The benchmark is also the `HOTPATH_BENCHMARK` value in the repository's `.github/workflows/*.yml`.

`validate-policy` and `init` need no token.

## Workflow: check or fix a change

Branch on the exit code, then read the JSON ("Output and exit codes"). Do not parse the human text of the pull request comment.

1. Push the commit. CI runs the benchmark and uploads the report, which takes as long as the benchmark job.
2. Wait for the report. Poll until it exits `0`, waiting about 30 seconds between attempts and giving up after about 30 minutes:

   ```bash
   hotpath cloud report --repo OWNER/NAME --benchmark NAME --commit $(git rev-parse HEAD) --no-payload
   ```

   `not_found` means the upload has not arrived yet. If it never arrives, check the CI run with `gh run list` / `gh run view` and the "Troubleshooting" table of https://hotpath.rs/ci_integration.
3. Read the judgement, as the "diff" section explains it:

   ```bash
   hotpath cloud diff --repo OWNER/NAME --benchmark NAME --commit $(git rev-parse HEAD)
   ```

4. Exit `0`: done, report the verdict and `dashboard_url` to the user. Exit `1`: read the regressions and broken budgets, fix the code, and repeat from step 1.

`--pr N` or `--id ID` select a report instead of `--commit`; `--advisory` and `--full` widen what `diff` lists.

Useful extracts:

```bash
# Regressions with the values that crossed the bar
hotpath cloud diff ... | jq '.result.sections[]? | .rows[] | select(.outcome == "regression")
  | {name, location, cells: [.cells[] | select(.crossed) | {column, base, head, change_percent}]}'

# Broken budgets with their messages
hotpath cloud diff ... | jq '.budgets.findings[] | {entity: .entity.name, actual, limit, message}'
```

## Fixing a regression

- Start from `location` of each regressed row and the `crossed` cells: an allocation regression points at new or larger allocations in that function, a `calls` / `count` change at extra work, `sql_per_request` at an N+1 query.
- Allocations are deterministic for a fixed workload, so an allocation change is real. Timing on shared CI runners is noisy; be careful attributing a small timing change to the code.
- A merged report (`meta.run` is `"merged"`: native timing + Callgrind instruction counts in one report) carries both. Its `icount` findings are the CPU verdict; the timing next to them is advisory, and on shared runners mostly noise unless the change is large and repeats across runs.
- An `icount` regression (Callgrind instruction counts) means the function executed more instructions: extra work, a different algorithm or code that is no longer inlined. To investigate one:
  - Read the changed code of the regressed function and its callees, and how far the count moved (`base`, `head`, `change_percent`).
  - Check the comment and `diff` for an `environment mismatch` note: counts only compare between matching Valgrind version, `rustc` and OS, so a mismatched section was not judged, which does not mean it is clean.
  - When possible, compare repeated runs of the same commit (re-run the job). How much counts vary depends on the workload, concurrent code most; a change within that spread may be noise.
  - More instructions is more work, not necessarily more elapsed time; say so when you report it, and do not claim a speed regression from the count alone.
  - Counts from a local run differ from CI's even for the same commit: compare a fix against a local run of the base commit, never against the CI numbers.
  - Raising `[functions.icount]` `min_percent_change` to make a result disappear is loosening the policy (see "Rules"). Raise it only with the user's agreement and from measured run-to-run variation.
- Profile locally to confirm before and after a fix: run the benchmark the CI workflow runs, with the same `--features`, and compare the printed hotpath report. For an `icount` regression that means running it under Valgrind as the workflow does, which needs x86_64 Linux: the merged pair (native run, then the Callgrind run with `HOTPATH_MERGE_REPORT`) when you also want timing, or only the Callgrind run when the counts are enough. Elsewhere, push the fix and read the next verdict.
- A broken budget names its rule's `message`, which usually says what the limit protects.

## Rules

- Never loosen the policy (`hotpath/*.toml`) or a budget to make a regression or a broken budget go away. Fix the code, or explain the trade-off to the user and let them decide.
- If the user asks to change the policy, fetch https://hotpath.rs/regression_policy (and https://hotpath.rs/performance_budgets for budgets), edit `hotpath/policy.toml` (or `hotpath/<benchmark>-policy.toml`) and check it with `hotpath cloud validate-policy` (`--benchmark NAME` for a benchmark's own file) before pushing. An upload with an invalid policy is refused.
- `hotpath cloud init` writes the default policy when the repository has none. It never replaces an existing file without `--force`; do not pass `--force` unless the user asks.
- `HOTPATH_API_TOKEN` is a credential: never print it, echo it, log it or commit it.
