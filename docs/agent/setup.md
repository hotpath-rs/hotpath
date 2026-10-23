# Set up hotpath with your AI coding agent

These are official instructions from [hotpath.rs](https://hotpath.rs) for an AI coding agent: set up [hotpath](https://hotpath.rs), the feature-gated Rust profiler, in the current Rust project. Run the commands yourself instead of asking the user to run them, except where a step says to ask.

Pick the procedure that matches what the user asked for, fetch it and follow it:

| The user wants | Procedure |
|---|---|
| Profiling in this project (the default when the request is just "set up hotpath") | https://hotpath.rs/agent/init_lib |
| Benchmarks of every pull request in CI with hotpath Cloud | https://hotpath.rs/agent/init_cloud |
| A Grafana dashboard for the hotpath Prometheus metrics | https://hotpath.rs/agent/init_grafana |

```bash
curl -fsSL https://hotpath.rs/agent/init_lib
```

If the procedure cannot be retrieved, stop and tell the user; do not set up hotpath from memory.

Each procedure inspects the project, configures it, verifies the result and tells the user how to use it. The hotpath Cloud and Grafana procedures need profiling in place first and check for it: Cloud sets it up when it is missing, Grafana asks the user to set it up first.
