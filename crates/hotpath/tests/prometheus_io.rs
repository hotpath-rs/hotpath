#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath-prometheus"))]
pub mod tests {
    use crate::common::example::Example;
    use crate::common::prometheus::{
        assert_histogram_family, assert_sample_lines_parse, scrape_until, series_value,
    };

    const METRICS_PORT: &str = "6785";
    const PROMETHEUS_PORT: &str = "6786";

    // cargo run -p test-io --example basic_io_sync --features hotpath-prometheus
    #[test]
    fn test_io_families() {
        let _running = Example::new("test-io", "basic_io_sync")
            .features("hotpath-prometheus")
            .env("HOTPATH_METRICS_PORT", METRICS_PORT)
            .env("HOTPATH_PROMETHEUS_PORT", PROMETHEUS_PORT)
            .env("TEST_SLEEP_SECONDS", "15")
            .spawn();

        // The `iter = true` wrappers are created last, so their presence means
        // the whole deterministic workload has been swept.
        let body = scrape_until(PROMETHEUS_PORT, 80, |body| {
            body.contains("label=\"itered\"")
        });

        for family in [
            "hotpath_io_ops_total",
            "hotpath_io_bytes_total",
            "hotpath_io_sampled_bytes_total",
            "hotpath_io_errors_total",
            "hotpath_io_op_seconds",
        ] {
            assert!(
                body.contains(&format!("# TYPE {family} ")),
                "missing family {family}, body:\n{body}"
            );
        }

        assert_sample_lines_parse(&body);

        assert_histogram_family(&body, "hotpath_io_op_seconds");

        // The failing reader errors on read, the flush-fail writer on flush.
        assert!(
            series_value(
                &body,
                "hotpath_io_errors_total",
                &["label=\"failing\"", "op=\"read\""]
            ) >= 1.0
        );
        assert!(
            series_value(
                &body,
                "hotpath_io_errors_total",
                &["label=\"flush-fail\"", "op=\"flush\""]
            ) >= 1.0
        );

        // `iter = true` wrappers get one entry per instance: two series
        // distinguished by the iter label.
        for iter in ["iter=\"0\"", "iter=\"1\""] {
            assert_eq!(
                series_value(
                    &body,
                    "hotpath_io_ops_total",
                    &["label=\"itered\"", "op=\"read\"", iter]
                ),
                1.0
            );
        }
    }
}
