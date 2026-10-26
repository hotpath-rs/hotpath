# Detect Rust performance regressions before they ship

<img src="{{#asset-hash images/hotpath-cloud-ferris.webp}}" alt="hotpath-cloud mascot, an angry crab on fire sitting on a cloud" class="ferris-img">

`hotpath-cloud` turns profiling data into automated performance reviews for Rust pull requests.

Compare each PR's benchmark results against its baseline to find functions that execute more CPU instructions, allocate more memory, or trigger additional SQL queries. Set performance budgets and get actionable feedback directly in GitHub PRs.

**Open beta:** Free to use. Public repositories will remain free. Paid plans for private repositories will be introduced after beta.

<div class="cta-row">
  <a class="agent-pill cta-link" target="_blank" href="/app">Get started</a>
  <a class="agent-pill cta-link cta-secondary" target="_blank" href="https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/drain/reports/01a124b3-d1f1-72ea-87ce-95057a276319/diff">See example performance diff</a>
</div>

The [GitHub Actions integration](ci_integration.md) compares benchmark results against a baseline and posts a PR comment whenever a significant regression or improvement is detected:

<img loading="lazy" src="{{#asset-hash images/cloud-pr-comment.webp}}" alt="hotpath-cloud pull request comment">

## Focus on performance changes that matter

Not every performance change should count as a regression. A [regression policy](regression_policy.md) defines how much each metric must change before it is flagged, filtering out insignificant changes. [Performance budgets](performance_budgets.md) enforce application-specific limits regardless of the baseline, such as a hot path that must never allocate or a cap on SQL queries per request.

```toml
# hotpath/policy.toml (excerpt)

# Flag a function whose allocations grow by 20% or more
[functions.alloc]
judged = true
min_percent_change = 20
metrics = ["avg", "total"]

# The hot path must never allocate, regardless of the baseline
[[functions.budgets]]
match = "my_crate::router::route"
alloc = { total = "0 B" }
message = "route() is on the hot path and must not allocate"

# Prevent N+1 calls: at most 5 SQL queries per request, on every route
[[server.budgets]]
match = "*"
timing = { sql_per_request = 5 }
message = "too many SQL queries per request, check for N+1 calls"
```

## Catch CPU regressions despite noisy CI

Reliable regression detection starts with repeatable measurements. Shared CI runners make wall-clock benchmarks noisy, potentially hiding small performance changes. For CPU-bound benchmarks with deterministic workloads, the recommended CI setup runs the benchmark under Valgrind [Callgrind](callgrind.md) and compares executed CPU instructions. In our own CI benchmarks, instruction counts typically vary by **less than 0.5%** between runs, even on shared GitHub runners.

Instruction counts measure computational work, not execution time: a higher count usually, but not always, means slower code. [Merged reports](ci_integration.md#profiling-modes) combine them with native timing, allocations, and other profiling metrics, so you can see what changed and where to look.

## Investigate and fix regressions with AI agents

The [`hotpath cloud`](agents_cli.md) CLI and the `hotpath-rs` [agent skill](https://github.com/hotpath-rs/hotpath/blob/main/skills/hotpath-rs/SKILL.md) give coding agents access to the same reports. An agent can analyze a performance diff, pinpoint the functions that regressed, and use the profiling data to guide optimizations:

<img loading="lazy" src="{{#asset-hash images/cloud-agent-diff.webp}}" alt="AI agent summarizing a hotpath-cloud diff">

Every uploaded report is persisted. The dashboard compares each report against its baseline, making it easy to dig into a regression and share the results with your team:

<img loading="lazy" src="{{#asset-hash images/cloud-dashboard-diff.webp}}" alt="hotpath-cloud dashboard diff page">

## Getting Started

`hotpath-cloud` supports `hotpath` version `0.29` or newer.

Both setups below use the hotpath GitHub App. It asks for minimal permissions: **no access to your source code**, only pull request write access to post comments, and checks to optionally report the status of each benchmark. The only data it gets from your CI is the benchmark reports your workflow uploads.

### AI Setup (Recommended)

The quickest way to set up hotpath-cloud is to let your own AI coding agent do it. Paste this prompt into your coding agent, inside your project repo:

```text
Set up hotpath-cloud CI in this Rust project using the instructions at https://hotpath.rs/agent/init_cloud
```

If the project does not use hotpath yet, the agent sets up profiling first. It then picks a benchmark with a fixed workload (asking you when the choice is not obvious), adds the `hotpath-cloud` feature, a [regression policy](regression_policy.md), a GitHub Actions workflow, and runs the benchmark locally to verify the report. For public repositories, it asks whether to benchmark pull requests from forks, which need a [separate setup](ci_integration.md#pull-requests-from-forks).

The agent does not commit, install the GitHub App or upload anything for you. Install the hotpath GitHub App from <a href="https://hotpath.rs/app" target="_blank" rel="noopener noreferrer">hotpath.rs/app</a> and merge the workflow: the first push to the default branch uploads the baseline, and from then on every pull request gets a performance comment.

Your agent remains in control: you review and approve edits through its regular permission prompts.

### Manual setup

Follow the [CI integration](ci_integration.md) guide to add the benchmark workflow by hand.

### README badge

Link your repository's hotpath-cloud page from its README:

[![hotpath-cloud](https://img.shields.io/endpoint?url=https://hotpath.rs/badge-cloud.json)](https://hotpath.rs/app/repos/hotpath-rs/hotpath)

```markdown
[![hotpath-cloud](https://img.shields.io/endpoint?url=https://hotpath.rs/badge-cloud.json)](https://hotpath.rs/app/repos/OWNER/NAME)
```

Replace `OWNER/NAME` with your GitHub repository. The page opens once the hotpath GitHub App is installed on the repository. Reports of a public repository are public by default; for a private repository the page opens only for users with access to it.
