#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath-prometheus"))]
pub mod tests {
    use crate::common::example::Example;
    use crate::common::prometheus::{assert_histogram_family, scrape_until, series_value};

    const METRICS_PORT: &str = "6783";
    const PROMETHEUS_PORT: &str = "6784";

    // cargo run -p test-all-features --example prometheus_flow --features hotpath-alloc,hotpath-prometheus
    #[test]
    fn test_alloc_families() {
        let _running = Example::new("test-all-features", "prometheus_flow")
            .features("hotpath-alloc,hotpath-prometheus")
            .env("HOTPATH_METRICS_PORT", METRICS_PORT)
            .env("HOTPATH_PROMETHEUS_PORT", PROMETHEUS_PORT)
            .env("TEST_SLEEP_SECONDS", "15")
            .spawn();

        // The measured allocating function runs exactly 50 times at the end
        // of the workload.
        let body = scrape_until(PROMETHEUS_PORT, 80, |body| {
            body.contains(
                "hotpath_function_calls_total{function=\"prometheus_flow::allocate_chunk\"} 50",
            )
        });

        // Timing families still render with the alloc feature on (the
        // TimingRaw-under-alloc path), alongside the alloc families.
        for family in [
            "hotpath_function_calls_total",
            "hotpath_function_duration_seconds",
            "hotpath_function_alloc_bytes_total",
            "hotpath_function_alloc_count_total",
            "hotpath_function_alloc_bytes",
            "hotpath_function_alloc_count",
        ] {
            assert!(
                body.contains(&format!("# TYPE {family} ")),
                "missing family {family}, body:\n{body}"
            );
        }

        for family in [
            "hotpath_function_alloc_bytes",
            "hotpath_function_alloc_count",
        ] {
            assert_histogram_family(&body, family);
        }

        // allocate_chunk allocates one 1024-byte vec per call, 50 calls.
        let function = "function=\"prometheus_flow::allocate_chunk\"";
        assert!(series_value(&body, "hotpath_function_alloc_bytes_total", &[function]) >= 51_200.0);
        assert!(series_value(&body, "hotpath_function_alloc_count_total", &[function]) >= 50.0);
        assert_eq!(
            series_value(&body, "hotpath_function_alloc_bytes_count", &[function]),
            50.0
        );
    }
}
