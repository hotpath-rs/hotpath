# Set up hotpath with your AI coding agent

These are official instructions from [hotpath.rs](https://hotpath.rs) for an AI coding agent: install the hotpath-rs skill, then set up hotpath in the current Rust project. Run the commands yourself instead of asking the user to run them, except where a step says to ask.

## 1. Install the hotpath-rs skill

The skill maps every hotpath task to the documentation page that covers it, so later sessions know where to look without these instructions.

```bash
npx -y skills add hotpath-rs/hotpath --global --yes
```

If `npx` is not available, or the install fails, tell the user and continue with step 2: the pages below are self-contained. After the install, tell the user to restart their agent session (or reload skills) so it picks the skill up.

## 2. Set up hotpath

Pick the page that matches what the user asked for, fetch it and follow it:

| The user wants | Page |
|---|---|
| Profiling in this project (the default when the request is just "set up hotpath") | https://hotpath.rs/agent/init_lib.md |
| Benchmarks of every pull request in CI with hotpath Cloud | https://hotpath.rs/agent/init_cloud.md |
| A Grafana dashboard for the hotpath Prometheus metrics | https://hotpath.rs/agent/init_grafana.md |

```bash
curl -sL https://hotpath.rs/agent/init_lib.md
```

The hotpath Cloud and Grafana setups need profiling in place first; their pages check for it and send you to the profiling setup when it is missing.
