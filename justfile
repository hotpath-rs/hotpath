default:
    @just --list

# Run benchmark for current state
bench:
    bash scripts/bench.sh

# Run benchmark for current state (meta profiler)
bench_meta:
    bash scripts/bench_meta.sh

# Run all tests
test_all:
    cargo run -p test-all-features --example all_noop
    cargo test --features hotpath --test guards -- --nocapture --test-threads=1
    cargo test --features hotpath --test user_metadata -- --nocapture --test-threads=1
    cargo test --features hotpath --test functions_timing -- --nocapture --test-threads=1
    cargo test --features hotpath --test locations -- --nocapture --test-threads=1
    cargo test --features hotpath --test unique_labels -- --nocapture --test-threads=1
    cargo test --features hotpath --test cdylib -- --nocapture --test-threads=1
    cargo test --features hotpath --test time_sampling -- --nocapture --test-threads=1
    cargo test --features hotpath --test metrics_auth -- --nocapture --test-threads=1
    cargo test --features hotpath --test functions_alloc -- --nocapture --test-threads=1
    cargo test --features hotpath --test functions_alloc_meta -- --nocapture --test-threads=1
    cargo test --features hotpath --test functions_cpu -- --nocapture --test-threads=1
    cargo test --features hotpath --test streams -- --nocapture --test-threads=1
    cargo test --features hotpath --test channels_crossbeam -- --nocapture --test-threads=1
    cargo test --features hotpath --test channels_ftc -- --nocapture --test-threads=1
    cargo test --features hotpath --test channels_asc -- --nocapture --test-threads=1
    cargo test --features hotpath --test channels_std -- --nocapture --test-threads=1
    cargo test --features hotpath --test channels_tokio -- --nocapture --test-threads=1
    cargo test --features hotpath --test channels_flume -- --nocapture --test-threads=1
    cargo test --features hotpath --test rw_lock_std -- --nocapture --test-threads=1
    cargo test --features hotpath --test rw_lock_parking_lot -- --nocapture --test-threads=1
    cargo test --features hotpath --test mutex_std -- --nocapture --test-threads=1
    cargo test --features hotpath --test mutex_parking_lot -- --nocapture --test-threads=1
    cargo test --features hotpath --test mutex_tokio -- --nocapture --test-threads=1
    cargo test --features hotpath --test mutex_async_lock -- --nocapture --test-threads=1
    cargo test --features hotpath --test threads -- --nocapture --test-threads=1
    cargo test --features hotpath --test tokio_runtime -- --nocapture --test-threads=1
    cargo test --features hotpath --test futures -- --nocapture --test-threads=1
    cargo test --features hotpath --test io -- --nocapture --test-threads=1
    cargo test --features hotpath --test io_redis -- --nocapture --test-threads=1
    cargo test --features hotpath --test http_reqwest -- --nocapture --test-threads=1
    cargo test --features hotpath --test http_ureq -- --nocapture --test-threads=1
    cargo test --features hotpath --test server_axum -- --nocapture --test-threads=1
    cargo test --features hotpath --test server_axum_route_scope -- --nocapture --test-threads=1
    cargo test --features hotpath --test server_axum_alloc -- --nocapture --test-threads=1
    cargo test --features hotpath --test sql_sqlite -- --nocapture --test-threads=1
    cargo test --features hotpath --test sql_pg -- --nocapture --test-threads=1
    cargo test --features hotpath --test sql_seaorm -- --nocapture --test-threads=1
    cargo test --features hotpath --test diesel -- --nocapture --test-threads=1
    cargo test --features hotpath --test diesel_pg -- --nocapture --test-threads=1
    cargo test --features hotpath --test toasty_sqlite -- --nocapture --test-threads=1
    cargo test --features hotpath --test toasty_pg -- --nocapture --test-threads=1
    cargo test --features hotpath --test debug -- --nocapture --test-threads=1
    cargo test --features hotpath-prometheus --test prometheus_metrics -- --nocapture --test-threads=1
    cargo test --features hotpath-prometheus --test prometheus_native -- --nocapture --test-threads=1
    cargo test --features hotpath-prometheus --test prometheus_subsystems -- --nocapture --test-threads=1
    cargo test --features hotpath-prometheus --test prometheus_flow -- --nocapture --test-threads=1
    cargo test --features hotpath-prometheus --test prometheus_io -- --nocapture --test-threads=1
    cargo test --features hotpath-prometheus --test prometheus_alloc -- --nocapture --test-threads=1
    cargo test --features hotpath-prometheus --test prometheus_system -- --nocapture --test-threads=1
    cargo test --features hotpath --test cloud_limit -- --nocapture --test-threads=1
    cargo test --features hotpath --test cloud_json_file -- --nocapture --test-threads=1
    cargo test --features hotpath --test cloud_upload_guard -- --nocapture --test-threads=1
    cargo test --features hotpath --test cloud_meta -- --nocapture --test-threads=1
    cargo test --features hotpath --test cloud_precision -- --nocapture --test-threads=1
    cargo test -p hotpath --features cloud --test cloud_cli

# Scrape with `docker compose up -d prometheus grafana`; on native Linux add HOTPATH_PROMETHEUS_HOST=0.0.0.0.
# Run the TUI in demo mode with the Prometheus exporter on port 6772.
demo:
    cargo run --bin hotpath --features tui,hotpath-alloc,hotpath-prometheus,demo,dev,hotpath-alloc-meta,hotpath-prometheus-meta -- console

# Open the demo Grafana dashboard fed by the native-histogram Prometheus.
grafana:
    open "http://localhost:3009/d/hotpath-functions" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-functions"

# Open the demo Grafana dashboard fed by the legacy (classic buckets only) Prometheus.
grafana-legacy:
    open "http://localhost:3009/d/hotpath-functions-legacy" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-functions-legacy"

# Open the sql / http / server Grafana dashboard fed by the native-histogram Prometheus.
grafana-web:
    open "http://localhost:3009/d/hotpath-web" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-web"

# Open the sql / http / server Grafana dashboard fed by the legacy (classic buckets only) Prometheus.
grafana-web-legacy:
    open "http://localhost:3009/d/hotpath-web-legacy" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-web-legacy"

# Open the per-route (sql / http drilldown) Grafana dashboard fed by the native-histogram Prometheus.
grafana-route:
    open "http://localhost:3009/d/hotpath-route" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-route"

# Open the per-route (sql / http drilldown) Grafana dashboard fed by the legacy (classic buckets only) Prometheus.
grafana-route-legacy:
    open "http://localhost:3009/d/hotpath-route-legacy" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-route-legacy"

# Open the locks / channels / streams Grafana dashboard fed by the native-histogram Prometheus.
grafana-flow:
    open "http://localhost:3009/d/hotpath-flow" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-flow"

# Open the locks / channels / streams Grafana dashboard fed by the legacy (classic buckets only) Prometheus.
grafana-flow-legacy:
    open "http://localhost:3009/d/hotpath-flow-legacy" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-flow-legacy"

# Open the locks / channels / streams Grafana dashboard fed by the meta Prometheus (hotpath profiling itself).
grafana-flow-meta:
    open "http://localhost:3009/d/hotpath-flow-meta" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-flow-meta"

# Open the locks / channels / streams Grafana dashboard fed by the legacy meta Prometheus.
grafana-flow-legacy-meta:
    open "http://localhost:3009/d/hotpath-flow-legacy-meta" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-flow-legacy-meta"

# Open the io / futures / alloc Grafana dashboard fed by the native-histogram Prometheus.
grafana-io:
    open "http://localhost:3009/d/hotpath-io" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-io"

# Open the io / futures / alloc Grafana dashboard fed by the legacy (classic buckets only) Prometheus.
grafana-io-legacy:
    open "http://localhost:3009/d/hotpath-io-legacy" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-io-legacy"

# Open the io / futures / alloc Grafana dashboard fed by the meta Prometheus (hotpath profiling itself).
grafana-io-meta:
    open "http://localhost:3009/d/hotpath-io-meta" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-io-meta"

# Open the io / futures / alloc Grafana dashboard fed by the legacy meta Prometheus.
grafana-io-legacy-meta:
    open "http://localhost:3009/d/hotpath-io-legacy-meta" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-io-legacy-meta"

# Open the threads / tokio / gauges Grafana dashboard fed by the native-histogram Prometheus.
grafana-system:
    open "http://localhost:3009/d/hotpath-system" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-system"

# Open the threads / tokio / gauges Grafana dashboard fed by the legacy (classic buckets only) Prometheus.
grafana-system-legacy:
    open "http://localhost:3009/d/hotpath-system-legacy" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-system-legacy"

# Open the threads / tokio / gauges Grafana dashboard fed by the meta Prometheus (hotpath profiling itself).
grafana-system-meta:
    open "http://localhost:3009/d/hotpath-system-meta" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-system-meta"

# Open the threads / tokio / gauges Grafana dashboard fed by the legacy meta Prometheus.
grafana-system-legacy-meta:
    open "http://localhost:3009/d/hotpath-system-legacy-meta" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-system-legacy-meta"

# Open the native-histogram Prometheus UI (started with `docker compose up -d prometheus`).
prometheus:
    open "http://localhost:9099" 2>/dev/null || xdg-open "http://localhost:9099"

# Open the legacy Prometheus 2.x UI (started with `docker compose up -d prometheus-legacy`).
prometheus-legacy:
    open "http://localhost:9098" 2>/dev/null || xdg-open "http://localhost:9098"

# Open the Grafana dashboard fed by the native-histogram Prometheus scraping the hotpath-meta exporter.
grafana-meta:
    open "http://localhost:3009/d/hotpath-functions-meta" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-functions-meta"

# Open the Grafana dashboard fed by the legacy Prometheus scraping the hotpath-meta exporter.
grafana-legacy-meta:
    open "http://localhost:3009/d/hotpath-functions-legacy-meta" 2>/dev/null || xdg-open "http://localhost:3009/d/hotpath-functions-legacy-meta"

# Open the native-histogram Prometheus UI scraping the hotpath-meta exporter.
prometheus-meta:
    open "http://localhost:9097" 2>/dev/null || xdg-open "http://localhost:9097"

# Open the legacy Prometheus 2.x UI scraping the hotpath-meta exporter.
prometheus-legacy-meta:
    open "http://localhost:9096" 2>/dev/null || xdg-open "http://localhost:9096"

# Wipe the gathered Prometheus data (native-histogram and legacy instances).
clean-prometheus:
    docker compose rm -sf prometheus prometheus-legacy
    docker compose up -d prometheus prometheus-legacy

# Wipe the gathered Prometheus data of the meta instances (native-histogram and legacy).
clean-prometheus-meta:
    docker compose rm -sf prometheus-meta prometheus-legacy-meta
    docker compose up -d prometheus-meta prometheus-legacy-meta

# Serve the mdbook docs locally with live reload (http://localhost:3000).
docs:
    cd docs && mdbook serve --open

# Fetch GitHub star badges locally for documentation
fetch-badges:
    #!/usr/bin/env bash
    set -euo pipefail
    DIR="docs/src/images"
    fetch() { sleep 2; echo "Fetching $2..."; curl -sL "https://img.shields.io/github/stars/${2}?style=social" -o "${DIR}/stars-${1}.svg"; }
    fetch PostHog-posthog PostHog/posthog
    fetch rustfs-rustfs rustfs/rustfs
    fetch apache-opendal apache/opendal
    fetch maplibre-martin maplibre/martin
    fetch marc2332-freya marc2332/freya
    fetch parseablehq-parseable parseablehq/parseable
    fetch hotpath-rs-hotpath hotpath-rs/hotpath

    echo "Badges saved to ${DIR}/"

# Publish in dependency order (optional deps included).
cargo-publish:
    cargo publish -p hotpath-drain-meta
    cargo publish -p hotpath-macros-meta
    cargo publish -p hotpath-meta
    cargo publish -p hotpath-drain
    cargo publish -p hotpath-macros
    cargo publish -p hotpath
