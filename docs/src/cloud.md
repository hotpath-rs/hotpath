# See what changed in your Rust app's performance

<img src="{{#asset-hash images/hotpath-cloud-ferris.webp}}" alt="hotpath-cloud mascot, an angry crab on fire sitting on a cloud" class="ferris-img">

hotpath-cloud turns profiling data into automated performance reviews for pull requests.

Compare benchmarks against your baseline to see which functions execute more CPU instructions, allocate more memory, or perform additional work. Set regression thresholds and performance budgets to focus on changes that matter.

Get actionable feedback directly in GitHub PRs, with detailed reports that developers and coding agents can use to investigate regressions.

> **Open beta:** free to use. Public repositories will remain free after beta.

To get started, log in at <a href="https://hotpath.rs/app" target="_blank" rel="noopener noreferrer">hotpath.rs/app</a>. The hotpath-rs GitHub App asks for minimal permissions: **no access to your code**, only pull request write access to post comments, and checks to optionally report the status of each benchmark. The only data it gets from your CI is the benchmark reports your workflow uploads. 

The [GitHub Actions integration](ci_integration.md) compares benchmark results against a baseline and posts a PR comment whenever a significant regression or improvement is detected:

<img loading="lazy" src="{{#asset-hash images/cloud-pr-comment.webp}}" alt="hotpath-cloud pull request comment">

## Reliable CPU performance comparisons in CI

Shared CI runners make timing measurements noisy. hotpath-cloud can use Valgrind [Callgrind](callgrind.md) to compare executed CPU instructions instead of relying on wall-clock timing alone.

For deterministic workloads, instruction counts provide a more repeatable signal of changes in computational work. [Combined reports](ci_integration.md#profiling-modes) also preserve native timing, allocations, and other profiling metrics, giving you a fuller picture of what changed.

You can define acceptable regression thresholds using a customizable [regression policy](regression_policy.md) and set fine-tuned [performance budgets](performance_budgets.md). This helps eliminate noisy alerts and ensures that warnings are emitted only when performance changes in a meaningful way.

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

The [`hotpath cloud`](agents_cli.md) CLI and the `hotpath-rs` [agent skill](https://github.com/hotpath-rs/hotpath/blob/main/skills/hotpath-rs/SKILL.md) bring the same feedback loop to coding agents. Agents can analyze benchmark results, identify regressions, and use the profiling data to guide performance optimization:

<img loading="lazy" src="{{#asset-hash images/cloud-agent-diff.webp}}" alt="AI agent summarizing a hotpath-cloud diff">

Benchmark reports are persisted in hotpath-cloud, making it easy to share results with your team and compare each report against its baseline:

<img loading="lazy" src="{{#asset-hash images/cloud-dashboard-diff.webp}}" alt="hotpath-cloud dashboard diff page">

<a href="https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/drain/reports/01a124b3-d1f1-72ea-87ce-95057a276319/diff" target="_blank" rel="noopener noreferrer">See this diff on hotpath.rs</a>

## Getting Started

hotpath-cloud supports `hotpath` version `0.28` or newer.

### AI Setup (Recommended)

The quickest way to set up hotpath-cloud is to let your own AI coding agent do it. Paste this prompt into your coding agent, inside your project repo:

```text
Set up hotpath-cloud CI in this Rust project using the instructions at https://hotpath.rs/agent/init_cloud
```

If the project does not use hotpath yet, the agent sets up profiling first. It then picks a benchmark with a fixed workload (asking you when the choice is not obvious), adds the `hotpath-cloud` feature (and, for a CPU-bound benchmark on `hotpath` `0.29.2` or newer, sets up [merged mode](ci_integration.md#profiling-modes): native timing + Callgrind instruction counts in one report), a [regression policy](regression_policy.md) file (an existing one is kept), a GitHub Actions benchmark workflow and the [README badge](#readme-badge), and runs the benchmark locally to verify the report. For public repositories, it asks whether pull requests from forks should be benchmarked too: they need a [separate setup](ci_integration.md#pull-requests-from-forks) with a relay workflow for security reasons.

The agent does not commit, install the GitHub App or upload anything for you. Install the hotpath GitHub App from <a href="https://hotpath.rs/app" target="_blank" rel="noopener noreferrer">hotpath.rs/app</a> and merge the workflow: the first push to the default branch uploads the baseline, and from then on every pull request gets a performance comment.

Your agent remains in control: you review and approve edits through its regular permission prompts.

### Manual setup

Follow the [CI integration](ci_integration.md) guide to add the benchmark workflow by hand.

### README badge

Link your repository's hotpath-cloud page from its README:

```markdown
[![hotpath-cloud](https://img.shields.io/endpoint?url=https://hotpath.rs/badge-cloud.json)](https://hotpath.rs/app/repos/OWNER/NAME)
```

Replace `OWNER/NAME` with your GitHub repository. The page opens once the hotpath GitHub App is installed on the repository. Reports of a public repository are public by default; for a private repository the page opens only for users with access to it.
