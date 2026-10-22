#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath-prometheus"))]
pub mod tests {
    use crate::common::example::Example;
    use crate::common::prometheus::{
        assert_histogram_family, assert_sample_lines_parse, scrape_until, series_value,
    };

    const METRICS_PORT: &str = "6798";
    const PROMETHEUS_PORT: &str = "6799";

    // cargo run -p test-all-features --example prometheus_flow --features hotpath-prometheus
    #[test]
    fn test_locks_channels_streams_families() {
        let _running = Example::new("test-all-features", "prometheus_flow")
            .features("hotpath-prometheus")
            .env("HOTPATH_METRICS_PORT", METRICS_PORT)
            .env("HOTPATH_PROMETHEUS_PORT", PROMETHEUS_PORT)
            .env("TEST_SLEEP_SECONDS", "15")
            .spawn();

        // The workload is deterministic; wait for its final state (50 received
        // messages, 7 stream items) so assertions run past every worker sweep.
        let body = scrape_until(PROMETHEUS_PORT, 80, |body| {
            body.contains("hotpath_stream_items_total")
                && body
                    .lines()
                    .any(|l| l.starts_with("hotpath_channel_received_total{") && l.ends_with(" 50"))
        });

        for family in [
            "hotpath_mutex_acquisitions_total",
            "hotpath_mutex_wait_seconds",
            "hotpath_mutex_acquire_seconds",
            "hotpath_rwlock_acquisitions_total",
            "hotpath_rwlock_wait_seconds",
            "hotpath_rwlock_acquire_seconds",
            "hotpath_channel_sent_total",
            "hotpath_channel_received_total",
            "hotpath_channel_instances_created_total",
            "hotpath_channel_instances_closed_total",
            "hotpath_channel_max_queue_size",
            "hotpath_channel_delay_seconds",
            "hotpath_stream_items_total",
            "hotpath_stream_instances_created_total",
            "hotpath_stream_instances_closed_total",
            "hotpath_future_polls_total",
            "hotpath_future_sampled_polls_total",
            "hotpath_future_poll_seconds_total",
        ] {
            assert!(
                body.contains(&format!("# TYPE {family} ")),
                "missing family {family}, body:\n{body}"
            );
        }

        assert_sample_lines_parse(&body);

        for family in [
            "hotpath_mutex_wait_seconds",
            "hotpath_mutex_acquire_seconds",
            "hotpath_rwlock_wait_seconds",
            "hotpath_rwlock_acquire_seconds",
            "hotpath_channel_delay_seconds",
        ] {
            assert_histogram_family(&body, family);
        }

        // Deterministic workload counts.
        assert_eq!(
            series_value(
                &body,
                "hotpath_channel_sent_total",
                &["label=\"work-queue\""]
            ),
            50.0
        );
        assert_eq!(
            series_value(
                &body,
                "hotpath_stream_items_total",
                &["label=\"number-stream\""]
            ),
            7.0
        );
        assert_eq!(
            series_value(
                &body,
                "hotpath_rwlock_acquisitions_total",
                &["op=\"write\""]
            ),
            5.0
        );
        assert_eq!(
            series_value(&body, "hotpath_rwlock_acquisitions_total", &["op=\"read\""]),
            15.0
        );
        assert_eq!(
            series_value(&body, "hotpath_future_polls_total", &["label=\"doubler\""]),
            20.0
        );

        // Sampling honesty: the histogram population never exceeds the
        // true acquisition count.
        let acquisitions = series_value(
            &body,
            "hotpath_mutex_acquisitions_total",
            &["label=\"counter\""],
        );
        let sampled = series_value(
            &body,
            "hotpath_mutex_wait_seconds_count",
            &["label=\"counter\""],
        );
        assert_eq!(acquisitions, 20.0);
        assert!(
            sampled <= acquisitions,
            "sampled {sampled} > acquisitions {acquisitions}"
        );
    }
}
