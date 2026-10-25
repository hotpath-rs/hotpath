# hotpath-cloud FAQ

## What data can the GitHub App access?

The hotpath GitHub App has **no access to your code**. It requests only two permissions:

- **Pull requests: write** - to post the performance comment on a pull request.
- **Checks: write** - to optionally report the result of each benchmark as a `hotpath / <benchmark>` check.

The only data hotpath-cloud receives from your CI is the benchmark reports that your workflow uploads. See [CI integration](ci_integration.md#setup) for details.

## Can I use hotpath-rs without hotpath-cloud?

Yes. The core `hotpath-rs` library remains open source, and hotpath-cloud is an optional integration built on top of it. Everything the profiler does locally works without an account.

## Is it free? Will it stay free for open source?

hotpath-cloud is free to use during the open beta. Paid plans will be introduced after the beta, but there will always be a free tier for public repositories.

## How long are benchmark reports kept?

The retention policy will be finalized after the beta. Every plan, including the free one, will keep at least a few hundred of the most recent benchmark reports.

## What happens to my CI if the hotpath-cloud backend is down?

Your CI keeps working, and you decide how strict it should be. With `HOTPATH_UPLOAD=enabled`, a failed upload only logs a warning and the job still passes, so a backend outage never blocks your pipeline. With `HOTPATH_UPLOAD=fail-on-error`, a failed upload fails the job.

Regressions and broken budgets never fail the job either. They are reported in the pull request comment, and they fail the pull request check only if your [regression policy](regression_policy.md#pull-request-check) says so. See [Failing the job](ci_integration.md#failing-the-job) for details.
