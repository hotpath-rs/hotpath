#!/usr/bin/env bash
# Runs every benchmark in agg-bench/bench.env in parallel, each after its own
# delay, and uploads its report. Run from the repository root.
set -uo pipefail
source agg-bench/bench.env
export AGG_CALLS
export HOTPATH_METRICS_SERVER_OFF=true
extra=0
if [ "${EVENT_ACTION:-}" = "synchronize" ]; then extra=$SYNC_EXTRA_DELAY; fi
mkdir -p "$RUNNER_TEMP/agg"
pids=()
for bench in $BENCHES; do
  key=${bench//-/_}
  bytes_var="${key}_BYTES"; delay_var="${key}_DELAY"
  bytes=${!bytes_var}; delay=$(( ${!delay_var:-0} + extra ))
  (
    sleep "$delay"
    echo "[$bench] uploading at $(date -u +%H:%M:%S) (delay ${delay}s, ${bytes} B x ${AGG_CALLS})"
    HOTPATH_BENCHMARK=$bench AGG_BYTES=$bytes \
      HOTPATH_UPLOAD_RESPONSE_PATH="$RUNNER_TEMP/agg/$bench.json" \
      agg-bench/target/release/agg-bench
    if [ -s "$RUNNER_TEMP/agg/$bench.json" ]; then
      echo "[$bench] accepted at $(date -u +%H:%M:%S): $(jq -c '{id, benchmark, verdict, dashboard_url}' "$RUNNER_TEMP/agg/$bench.json")"
    else
      echo "[$bench] NOT accepted at $(date -u +%H:%M:%S)"; exit 1
    fi
  ) &
  pids+=($!)
done
status=0
for pid in "${pids[@]}"; do wait "$pid" || status=1; done
exit $status
