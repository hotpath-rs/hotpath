#!/usr/bin/env bash
set -euo pipefail

RELEASE_FLAG=""
if [ "${HOTPATH_BENCH_RELEASE:-}" = "true" ]; then
    RELEASE_FLAG="--release"
fi
BENCH_CMD="cargo run $RELEASE_FLAG -p hotpath --features=tui,hotpath-alloc --bin hotpath"

mkdir -p tmp

REF=$(git rev-parse --short HEAD)
BRANCH=$(git rev-parse --abbrev-ref HEAD)
LABEL="$BRANCH ($REF)"
OUTPUT="tmp/bench.txt"
OUTPUT="${HOTPATH_OUTPUT_PATH:-$OUTPUT}"

bench_env=(
    HOTPATH_TUI_TAB=${HOTPATH_TUI_TAB:-1}
    HOTPATH_TUI_AUTO_EXPAND_LOGS=${HOTPATH_TUI_AUTO_EXPAND_LOGS:-1}
    HOTPATH_TUI_REFRESH_INTERVAL_MS=${HOTPATH_TUI_REFRESH_INTERVAL_MS:-10}
    HOTPATH_REPORT='functions-timing,functions-alloc,threads'
    HOTPATH_OUTPUT_PATH="$OUTPUT"
    HOTPATH_SHUTDOWN_MS=5000
    HOTPATH_TIMEOUT_MS=5000
    HOTPATH_EXCLUDE_WRAPPER=true
    HOTPATH_REPORT_LABEL="$LABEL"
    RUSTFLAGS='--cfg tokio_unstable'
)

echo "==> Running: ${bench_env[*]} $BENCH_CMD"
env "${bench_env[@]}" $BENCH_CMD

reset

echo ""
echo "==> Report for: $LABEL"
echo ""
cat "$OUTPUT"
