#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath-prometheus"))]
pub mod tests {
    use crate::common::example::Example;
    use crate::common::prometheus::{assert_sample_lines_parse, scrape_until, series_value};

    const METRICS_PORT: &str = "6779";
    const PROMETHEUS_PORT: &str = "6780";

    // cargo run -p test-all-features --example prometheus_system --features hotpath-prometheus
    #[test]
    fn test_threads_tokio_gauge_families() {
        let _running = Example::new("test-all-features", "prometheus_system")
            .features("hotpath-prometheus")
            .env("HOTPATH_METRICS_PORT", METRICS_PORT)
            .env("HOTPATH_PROMETHEUS_PORT", PROMETHEUS_PORT)
            .env("TEST_SLEEP_SECONDS", "20")
            .spawn();

        // Wait for one tokio runtime sample (1s interval), two thread monitor
        // samples (cpu_percent needs a delta between samples, so the
        // cpu_percent families are absent from the first one) covering the
        // named busy thread, and the gauges.
        let body = scrape_until(PROMETHEUS_PORT, 80, |body| {
            body.contains("hotpath_tokio_workers")
                && body.contains("name=\"busy-worker\"")
                && body.contains("hotpath_thread_cpu_percent_max")
                && body.contains("hotpath_gauge{")
        });

        for family in [
            "hotpath_threads",
            "hotpath_thread_cpu_seconds_total",
            "hotpath_thread_cpu_percent_max",
            "hotpath_tokio_workers",
            "hotpath_tokio_alive_tasks",
            "hotpath_tokio_global_queue_depth",
            "hotpath_tokio_worker_parks_total",
            "hotpath_tokio_worker_busy_seconds_total",
            "hotpath_gauge",
            "hotpath_gauge_min",
            "hotpath_gauge_max",
            "hotpath_gauge_updates_total",
        ] {
            assert!(
                body.contains(&format!("# TYPE {family} ")),
                "missing family {family}, body:\n{body}"
            );
        }

        assert_sample_lines_parse(&body);

        assert_eq!(series_value(&body, "hotpath_tokio_workers", &[]), 2.0);
        assert_eq!(
            series_value(&body, "hotpath_gauge", &["key=\"test-gauge\""]),
            42.0
        );
        assert_eq!(
            series_value(&body, "hotpath_gauge", &["key=\"queue-depth\""]),
            12.0
        );
        assert_eq!(
            series_value(
                &body,
                "hotpath_gauge_updates_total",
                &["key=\"queue-depth\""]
            ),
            3.0
        );

        // The busy-spinning named thread accumulated user CPU time.
        assert!(
            series_value(
                &body,
                "hotpath_thread_cpu_seconds_total",
                &["name=\"busy-worker\"", "mode=\"user\""]
            ) > 0.0
        );
        assert!(series_value(&body, "hotpath_threads", &[]) >= 3.0);
    }
}
