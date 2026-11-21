# Detect Rust performance regressions before they ship

<img src="{{#asset-hash images/hotpath-cloud-ferris.webp}}" alt="hotpath-cloud mascot, an angry crab on fire sitting on a cloud" class="ferris-img ferris-img-cloud">

Catch CPU, memory allocation, and SQL regressions automatically in GitHub pull requests. See exactly what changed before merging.

<div class="cloud-cta">
  <div class="cloud-cta-offer">
    <p class="cloud-cta-offer-title">We'll set it up for you.</p>
    <p>Install the GitHub App and select a public Rust repository. We'll prepare a ready-to-merge PR with <code>hotpath-cloud</code> configured. All you need to do is review and merge.</p>
  </div>
  <a class="agent-pill cta-link" href="https://github.com/apps/hotpath-rs/installations/new" target="_blank" rel="noopener" onclick="hotpathTrack('github_app_install_click', 'cloud_hero')">
    <svg class="agent-pill-copy" viewBox="0 0 24 24" aria-hidden="true" fill="currentColor"><path d="M12 .3a12 12 0 0 0-3.8 23.4c.6.1.8-.3.8-.6v-2.2c-3.3.7-4-1.4-4-1.4-.6-1.4-1.4-1.8-1.4-1.8-1-.7.1-.7.1-.7 1.2.1 1.8 1.2 1.8 1.2 1 1.8 2.8 1.3 3.5 1 .1-.8.4-1.3.7-1.6-2.7-.3-5.5-1.3-5.5-6 0-1.2.5-2.3 1.3-3.1-.2-.4-.6-1.6 0-3.2 0 0 1-.3 3.4 1.2a11.5 11.5 0 0 1 6 0c2.3-1.5 3.3-1.2 3.3-1.2.6 1.6.2 2.8.1 3.2.8.8 1.3 1.9 1.3 3.2 0 4.6-2.8 5.6-5.5 5.9.5.4.9 1.1.9 2.2v3.3c0 .3.1.7.8.6A12 12 0 0 0 12 .3"/></svg>
    <span>Install GitHub App</span>
  </a>
  <p class="cloud-cta-meta"><a href="/pricing">Free for public repositories</a> · <a href="/cloud_faq#what-data-can-the-github-app-access">No source code access</a></p>
  <p class="cloud-cta-alt">Prefer to set things up yourself or with an AI coding agent? Read on for step-by-step instructions.</p>
</div>

<script>
// Optional Ahrefs Web Analytics event (the script is often blocked), it must never affect the link.
function hotpathTrack(name, placement) {
  try {
    window.AhrefsAnalytics?.sendEvent(name, { props: { placement: placement } });
  } catch (_) {}
}
</script>

## See what hotpath-cloud adds to your pull requests

When a pull request changes performance in a meaningful way, the [GitHub Actions integration](ci_integration.md) posts a comment comparing it against the baseline:

<img loading="lazy" src="{{#asset-hash images/cloud-pr-comment.webp}}" alt="hotpath-cloud pull request comment">

<a href="https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/drain/reports/01a1846c-6da4-7cf6-a648-275e812a76b9/diff" target="_blank" rel="noopener">See an example performance diff on the dashboard →</a>

Want to understand how it works under the hood? Read [Rust Benchmarking: How to Detect Performance Regressions in CI](/blog/rust-performance-benchmarking) for real benchmark results and practical advice on detecting regressions in CI.

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

Reliable regression detection starts with repeatable measurements. Shared CI runners make wall-clock benchmarks noisy, potentially hiding small performance changes. For CPU-bound benchmarks with deterministic workloads, the recommended CI setup runs the benchmark under Valgrind [Callgrind](callgrind.md) and compares executed CPU instructions. In our own CI benchmarks, instruction counts typically vary by **less than 0.5%** between runs, even on shared GitHub runners ([see the 100-run analysis](/blog/rust-performance-benchmarking#counting-cpu-instructions-with-callgrind)).

Instruction counts measure computational work, not execution time: a higher count usually, but not always, means slower code. [Merged reports](ci_integration.md#profiling-modes) combine them with native timing, allocations, and other profiling metrics, so you can see what changed and where to look.

## Investigate and fix regressions with AI agents

The [`hotpath cloud`](agents_cli.md) CLI, the `hotpath-rs` [agent skill](https://github.com/hotpath-rs/hotpath/blob/main/skills/hotpath-rs/SKILL.md) and the [hotpath-cloud MCP server](mcp_api.md) give coding agents and AI assistants access to the same reports. An agent can analyze a performance diff, pinpoint the functions that regressed, and use the profiling data to guide optimizations:

<img loading="lazy" src="{{#asset-hash images/cloud-agent-diff.webp}}" alt="AI agent summarizing a hotpath-cloud diff">

Every uploaded report is persisted. The dashboard compares each report against its baseline, making it easy to dig into a regression and share the results with your team:

<img loading="lazy" src="{{#asset-hash images/cloud-dashboard-diff.webp}}" alt="hotpath-cloud dashboard diff page">

## Getting Started

`hotpath-cloud` requires `hotpath` version `0.30.0` or newer.

### Install the GitHub App (recommended for public repositories)

Install the GitHub App and select a public Rust repository. We'll personally prepare a ready-to-merge PR with `hotpath-cloud` configured. All you need to do is review and merge.

<div class="cloud-install">
  <a class="agent-pill cta-link" href="https://github.com/apps/hotpath-rs/installations/new" target="_blank" rel="noopener" onclick="hotpathTrack('github_app_install_click', 'getting_started')">
    <svg class="agent-pill-copy" viewBox="0 0 24 24" aria-hidden="true" fill="currentColor"><path d="M12 .3a12 12 0 0 0-3.8 23.4c.6.1.8-.3.8-.6v-2.2c-3.3.7-4-1.4-4-1.4-.6-1.4-1.4-1.8-1.4-1.8-1-.7.1-.7.1-.7 1.2.1 1.8 1.2 1.8 1.2 1 1.8 2.8 1.3 3.5 1 .1-.8.4-1.3.7-1.6-2.7-.3-5.5-1.3-5.5-6 0-1.2.5-2.3 1.3-3.1-.2-.4-.6-1.6 0-3.2 0 0 1-.3 3.4 1.2a11.5 11.5 0 0 1 6 0c2.3-1.5 3.3-1.2 3.3-1.2.6 1.6.2 2.8.1 3.2.8.8 1.3 1.9 1.3 3.2 0 4.6-2.8 5.6-5.5 5.9.5.4.9 1.1.9 2.2v3.3c0 .3.1.7.8.6A12 12 0 0 0 12 .3"/></svg>
    <span>Install GitHub App</span>
  </a>
  <p class="cloud-cta-meta"><a href="/pricing">Free for public repositories</a> · <a href="/cloud_faq#what-data-can-the-github-app-access">No source code access</a></p>
</div>

The GitHub App asks for minimal permissions: **no access to your source code**, only pull request write access to post comments, and checks to optionally report the status of each benchmark. The only data it gets from your CI is the benchmark reports your workflow uploads.

### Prefer to configure it yourself?

You can also set up `hotpath-cloud` with your AI coding agent or by following the manual integration guide. Our hands-on setup covers public Rust repositories; for a private repository, use one of these self-service setups. Both still need the [GitHub App installed](https://github.com/apps/hotpath-rs/installations/new).

#### AI setup

Paste this prompt into your coding agent, inside your project repo:

```text
Set up hotpath-cloud CI in this Rust project using the instructions at https://hotpath.rs/agent/init_cloud
```

If the project does not use hotpath yet, the agent sets up profiling first. It then picks a benchmark with a fixed workload (asking you when the choice is not obvious), adds the `hotpath-cloud` feature, a [regression policy](regression_policy.md), a GitHub Actions workflow, and runs the benchmark locally to verify the report. For public repositories, it asks whether to benchmark pull requests from forks, which need a [separate setup](ci_integration.md#pull-requests-from-forks).

The agent does not commit, install the GitHub App or upload anything for you. [Install the hotpath GitHub App](https://github.com/apps/hotpath-rs/installations/new) and merge the workflow: the first push to the default branch uploads the baseline, and from then on every pull request gets a performance comment.

Your agent remains in control: you review and approve edits through its regular permission prompts.

#### Manual setup

Follow the [CI integration](ci_integration.md) guide to add the benchmark workflow by hand.

### README badge

Link your repository's hotpath-cloud page from its README:

[![hotpath-cloud](https://img.shields.io/endpoint?url=https://hotpath.rs/badge-cloud.json)](https://hotpath.rs/app/repos/hotpath-rs/hotpath)

```markdown
[![hotpath-cloud](https://img.shields.io/endpoint?url=https://hotpath.rs/badge-cloud.json)](https://hotpath.rs/app/repos/OWNER/NAME)
```

Replace `OWNER/NAME` with your GitHub repository. The page opens once the hotpath GitHub App is installed on the repository. Reports of a public repository are public by default; for a private repository the page opens only for users with access to it.
