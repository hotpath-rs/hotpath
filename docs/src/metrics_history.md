# Benchmark metrics history

hotpath-cloud builds a performance history from every benchmark report your CI uploads. You can follow any metric across the commits of a branch or a pull request:

- CPU instruction counts measured with [Callgrind](callgrind.md),
- wall-clock execution time,
- [memory allocations](memory_profiling.md),
- the same metrics for individual functions labeled with `#[hotpath::measure(label = "...")]`.

A single report tells you how a commit performed, and a [diff](ci_integration.md) compares it with one baseline. The history shows the whole trend at once: when a metric moved, by how much, and how much it fluctuates when nothing changed.

## Not every metric is equally stable

To measure CI noise, we pushed 100 empty commits to a [pull request](https://github.com/hotpath-rs/hotpath/pull/836) in the `hotpath-rs` repository. Each commit ran the same `drain` benchmark on a GitHub-hosted x86 runner, with identical code every time. Here is <a href="https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/drain/history?scope=pr&pr=836&other_pr=&branch=main&source=totals&metric=elapsed" target="_blank" rel="noopener">its wall time</a> across those runs:

<img loading="lazy" src="{{#asset-hash images/history-drain-wall-time.png}}" alt="hotpath-cloud benchmark history of the drain benchmark's wall time across 100 identical x86 runs: median 68.39 ms, min 38.54 ms, max 98.94 ms, spread 156.74%, CV 17.07%">

The slowest run took 156.74% longer than the fastest. Over the same runs, <a href="https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/drain/history?scope=pr&pr=836&other_pr=&branch=main&source=totals&metric=icount" target="_blank" rel="noopener">the CPU instruction count</a> varied by only 0.13%:

<img loading="lazy" src="{{#asset-hash images/history-drain-icount.png}}" alt="hotpath-cloud benchmark history of the drain benchmark's total icount across 100 identical x86 runs: median 419,849,348, min 419,627,104, max 420,159,511, spread 0.13%, CV 0.02%">

<a href="https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/drain/history?scope=pr&pr=836&other_pr=&branch=main&source=totals&metric=allocated&precise=true" target="_blank" rel="noopener">Allocated bytes</a> were identical in every run.

Since the code never changed, every difference between these runs is measurement noise. A 5% wall-time regression would be lost in it, while a 1% change in instruction count would stand out. These results hold for this benchmark on these runners. Multithreaded code, I/O or nondeterministic workloads can make instruction counts and allocations vary too. Instruction counts also miss effects such as cache locality, so they don't replace timing. [Rust Benchmarking: How to Detect Performance Regressions in CI](/blog/rust-performance-benchmarking) has the full analysis, including ARM runners and existing Criterion benchmarks.

## Using the history

Open a benchmark in the hotpath-cloud dashboard and click **Performance history**. Pull request comments and report pages link to the history of their pull request.

1. Choose a branch or a pull request.
2. Pick a run total, such as instruction count, wall time or allocated bytes, or drill down into a labeled function's metric.
3. Read the trend and the summary above the chart: the median, the extremes and the spread. Each point links to its full report.

On a branch, the chart mixes real performance changes with noise. A pull request with repeated runs of unchanged code, like the one above, shows the noise on its own. Use that observed noise as a starting point when you choose [regression policy](regression_policy.md) thresholds or [performance budgets](performance_budgets.md). A threshold inside the noise flags changes that never happened. Treat the observed spread as an estimate from a finite sample, not a guaranteed bound.

The same reports are available as JSON through the [`hotpath cloud` CLI](agents_cli.md) and the [API and MCP server](mcp_api.md), for analysis the charts don't cover.
