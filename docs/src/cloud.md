# hotpath Cloud - Performance regression feedback for developers and AI agents

<img src="{{#asset-hash images/hotpath-cloud-ferris.webp}}" alt="hotpath Cloud mascot, an angry crab on fire sitting on a cloud" class="ferris-img">

`hotpath Cloud` turns `hotpath-rs` profiling data into automated PR performance checks. It analyzes performance signals and helps developers and coding agents detect meaningful regressions before they are merged.

**Open beta:** free to use. Public repositories will remain free after beta. See [Pricing](#pricing).

To get started, log in at <a href="https://hotpath.rs/app" target="_blank" rel="noopener noreferrer">hotpath.rs/app</a>. The hotpath-rs GitHub App asks for minimal permissions: **no access to your code**, only pull request write access to post comments, and checks to optionally report the status of each benchmark. The only data it gets from your CI is the benchmark reports your workflow uploads. 

The [GitHub Actions integration](ci_integration.md) compares benchmark results against a baseline and posts a PR comment whenever a significant regression or improvement is detected:

<img loading="lazy" src="{{#asset-hash images/cloud-pr-comment.webp}}" alt="hotpath Cloud pull request comment">

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

The [`hotpath cloud`](agents_cli.md) CLI and dedicated [AI skill](https://github.com/pawurb/hotpath-rs/blob/main/skills/hotpath_cloud/SKILL.md) bring the same feedback loop to coding agents. Agents can analyze benchmark results, identify regressions, and use the profiling data to guide performance optimization:

<img loading="lazy" src="{{#asset-hash images/cloud-agent-diff.webp}}" alt="AI agent summarizing a hotpath Cloud diff">

Benchmark reports are persisted in `hotpath Cloud`, making it easy to share results with your team and compare each report against its baseline:

<img loading="lazy" src="{{#asset-hash images/cloud-dashboard-diff.webp}}" alt="hotpath Cloud dashboard diff page">

<a href="https://hotpath.rs/app/repos/pawurb/hotpath-rs/benchmarks/drain/reports/01a124b3-d1f1-72ea-87ce-95057a276319/diff" target="_blank" rel="noopener noreferrer">See this diff on hotpath.rs</a>

## Getting Started

`hotpath Cloud` supports `hotpath` version `0.28` or newer.

### AI Setup (Recommended)

The quickest way to set up `hotpath Cloud` is to let your own AI coding agent do it. Install the `hotpath` CLI and run `init-ci` inside your project repo:

```bash
cargo install hotpath --version '^{{HOTPATH_VERSION}}'
hotpath init-ci --agent claude # or --agent codex / --agent opencode
```

`hotpath init-ci` downloads the [hotpath_init_ci agent skill](https://github.com/pawurb/hotpath-rs/blob/main/skills/hotpath_init_ci/SKILL.md) from GitHub and starts your installed Claude Code, Codex or OpenCode with it as setup instructions. The agent adds the `hotpath-cloud` feature, a [regression policy](regression_policy.md) file and a GitHub Actions benchmark workflow, so every pull request gets a performance comment.

Use `--forks` for public repositories that want to benchmark pull requests from forks. Fork pull requests need a [separate setup](ci_integration.md#pull-requests-from-forks) for security reasons:

```bash
hotpath init-ci --agent claude --forks
```

For private repositories, or if you don't need benchmarks on pull requests from forks, run `hotpath init-ci` without `--forks`.

Your agent remains in control: you review and approve edits through its regular permission prompts. Requires `curl` and the `claude`, `codex` or `opencode` CLI on `PATH`.

### Manual setup

Follow the [CI integration](ci_integration.md) guide to add the benchmark workflow by hand.

## Pricing

`hotpath Cloud` is free during the open beta. Public repositories will remain free. Paid plans for private repositories and optional features will be introduced later.
