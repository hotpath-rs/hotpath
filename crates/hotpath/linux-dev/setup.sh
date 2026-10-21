#!/usr/bin/env bash
# Build image, relax host sysctls, recreate container, pre-build hotpath-samply,
# and exec into a bash shell. Run from repo root.
set -euo pipefail

docker build -f crates/hotpath/linux-dev/Dockerfile -t hotpath-linux .

docker run --rm --privileged --pid=host justincormack/nsenter1 /bin/sh -c \
    'sysctl -w kernel.perf_event_paranoid=-1; sysctl -w kernel.kptr_restrict=0'

docker rm -f hotpath-linux 2>/dev/null || true

docker run -d --name hotpath-linux \
    --cap-add=SYS_PTRACE --cap-add=SYS_ADMIN --cap-add=PERFMON \
    --security-opt seccomp=unconfined \
    -v "$PWD":/work -w /work \
    -e CARGO_TARGET_DIR=/work/target-linux \
    -e HOTPATH_SAMPLY_WRAPPER_BIN=/work/target-linux/profiling/hotpath-samply \
    hotpath-linux sleep infinity

docker exec hotpath-linux \
    cargo build -p hotpath --bin hotpath-samply \
        --features hotpath-cpu --profile profiling

docker exec -it hotpath-linux bash
