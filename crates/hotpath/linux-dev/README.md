# Debugging `hotpath-cpu` on Linux

Persistent Ubuntu container with `samply` + `perf` + Rust toolchain for
iterating on `hotpath-cpu` from macOS (or any non-Linux host).

## One-liner setup

Build image, relax host sysctls, recreate container, pre-build wrapper, and
exec into bash. Run from repo root:

```bash
./crates/hotpath/linux-dev/setup.sh
```

`perf_event_paranoid=-1` enables hardware events for non-root; `kptr_restrict=0`
resolves kernel symbols. Both reset on Docker Desktop VM restart - re-run the
script.

`HOTPATH_SAMPLY_WRAPPER_BIN` baked into container env so every `cargo run` of
an example finds the wrapper. Pre-built once during setup; rebuilds automatically
when its source changes.

## Build image

```bash
docker build -f crates/hotpath/linux-dev/Dockerfile -t hotpath-linux .
```

## Host kernel setup (per VM boot)

`samply` needs `kernel.perf_event_paranoid <= 1`. Lives on **host kernel**;
`/proc/sys` is read-only inside containers, so cannot be set from the
Dockerfile or `docker exec`.

Bare-metal Linux:

```bash
echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid
```

Docker Desktop (host kernel = LinuxKit VM):

```bash
docker run --rm --privileged --pid=host justincormack/nsenter1 \
  /sbin/sysctl -w kernel.perf_event_paranoid=1
```

Resets on VM restart.

## Start persistent container

```bash
docker run -d --name hotpath-linux \
  --cap-add=SYS_PTRACE --cap-add=PERFMON \
  --security-opt seccomp=unconfined \
  -v "$PWD":/work -w /work \
  -e CARGO_TARGET_DIR=/work/target-linux \
  hotpath-linux sleep infinity
```

`CARGO_TARGET_DIR=/work/target-linux` keeps container builds separate from
host `target/` (different arch).

Log in:

```bash
docker exec -it hotpath-linux bash
```

## Run CPU smoke test

`cargo run` of an example does **not** build sibling bins, so build the
`hotpath-samply` wrapper first and point `HOTPATH_SAMPLY_WRAPPER_BIN` at it:

```bash
docker exec -it hotpath-linux bash -lc '
  cargo build -p hotpath --bin hotpath-samply --features hotpath-cpu --profile profiling &&
  HOTPATH_SAMPLY_WRAPPER_BIN=/work/target-linux/profiling/hotpath-samply \
    cargo run -p test-tokio-async --example cpu_basic \
      --features "hotpath-cpu" --profile profiling
'
```

A working run prints both the `timing` table and a `functions-cpu` table
attributing samples to `heavy_work` / `light_work`.

## Suppressing `[1]+ Stopped` shell noise

`samply` periodically `SIGSTOP`s the target to read `/proc/<pid>/maps`
coherently. The shell sees the stop and prints `[1]+ Stopped ...`, delaying
output. SIGSTOP is uncatchable.

Workaround: run under `setsid -w` (new session, detached from controlling
TTY):

```bash
setsid -w cargo run -p test-tokio-async --example cpu_basic \
  --features='hotpath-cpu' --profile profiling
```
