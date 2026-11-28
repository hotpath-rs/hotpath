# Getting Started

`hotpath-cloud` requires `hotpath` version `0.30.0` or newer.

## Install the GitHub App

Start by installing the GitHub App on your repository. It posts the performance comments on your pull requests.

<div class="cloud-install">
  <a class="agent-pill cta-link" href="https://github.com/apps/hotpath-rs/installations/new" target="_blank" rel="noopener" onclick="hotpathTrack('github_app_install_click', 'getting_started')">
    <svg class="agent-pill-copy" viewBox="0 0 24 24" aria-hidden="true" fill="currentColor"><path d="M12 .3a12 12 0 0 0-3.8 23.4c.6.1.8-.3.8-.6v-2.2c-3.3.7-4-1.4-4-1.4-.6-1.4-1.4-1.8-1.4-1.8-1-.7.1-.7.1-.7 1.2.1 1.8 1.2 1.8 1.2 1 1.8 2.8 1.3 3.5 1 .1-.8.4-1.3.7-1.6-2.7-.3-5.5-1.3-5.5-6 0-1.2.5-2.3 1.3-3.1-.2-.4-.6-1.6 0-3.2 0 0 1-.3 3.4 1.2a11.5 11.5 0 0 1 6 0c2.3-1.5 3.3-1.2 3.3-1.2.6 1.6.2 2.8.1 3.2.8.8 1.3 1.9 1.3 3.2 0 4.6-2.8 5.6-5.5 5.9.5.4.9 1.1.9 2.2v3.3c0 .3.1.7.8.6A12 12 0 0 0 12 .3"/></svg>
    <span>Install GitHub App</span>
  </a>
  <p class="cloud-cta-meta"><a href="/pricing">Free for public repositories</a> · <a href="/cloud_faq#what-data-can-the-github-app-access">No source code access</a></p>
</div>

<script>
// Optional Ahrefs Web Analytics event (the script is often blocked), it must never affect the link.
function hotpathTrack(name, placement) {
  try {
    window.AhrefsAnalytics?.sendEvent(name, { props: { placement: placement } });
  } catch (_) {}
}
</script>

The GitHub App asks for minimal permissions: **no access to your source code**, only pull request write access to post comments, and checks to optionally report the status of each benchmark. The only data it gets from your CI is the benchmark reports your workflow uploads.

## AI setup (recommended)

Paste this prompt into your coding agent, inside your project repo:

```text
Set up hotpath-cloud CI integration in this Rust project using the instructions at https://hotpath.rs/agent/init_cloud
```

If the project does not use hotpath yet, the agent sets up profiling first. It then picks a benchmark with a fixed workload (asking you when the choice is not obvious), adds the `hotpath-cloud` feature, a [regression policy](regression_policy.md), a GitHub Actions workflow, and runs the benchmark locally to verify the report. For public repositories, it asks whether to benchmark pull requests from forks, which need a [separate setup](ci_integration.md#pull-requests-from-forks).

Your agent remains in control: you review and approve edits through its regular permission prompts. It does not commit or upload anything for you. Merge the workflow: the first push to the default branch uploads the baseline, and from then on every pull request gets a performance comment.

## Manual setup

Prefer to configure it without an agent? Follow the [CI integration](ci_integration.md) guide to add the benchmark workflow by hand, with the GitHub App installed as above.

## README badge

Link your repository's hotpath-cloud page from its README:

[![hotpath-cloud](https://img.shields.io/endpoint?url=https://hotpath.rs/badge-cloud.json)](https://hotpath.rs/app/repos/hotpath-rs/hotpath)

```markdown
[![hotpath-cloud](https://img.shields.io/endpoint?url=https://hotpath.rs/badge-cloud.json)](https://hotpath.rs/app/repos/OWNER/NAME)
```

Replace `OWNER/NAME` with your GitHub repository. The page opens once the hotpath GitHub App is installed on the repository. Reports of a public repository are public by default; for a private repository the page opens only for users with access to it.
