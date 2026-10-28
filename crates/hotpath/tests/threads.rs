#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use {hotpath::json::JsonThreadsList, std::time::Duration};

    use crate::common::assert_contains_all;
    use crate::common::example::{poll_endpoint, Example};

    // HOTPATH_METRICS_PORT=6775 TEST_SLEEP_SECONDS=10 cargo run -p test-tokio-async --example basic --features hotpath
    #[test]
    fn test_threads_endpoint() {
        let _running = Example::new("test-tokio-async", "basic")
            .env("HOTPATH_METRICS_PORT", "6775")
            .env("TEST_SLEEP_SECONDS", "10")
            .spawn();

        let json_text = poll_endpoint(
            "http://localhost:6775/threads",
            30,
            Duration::from_millis(1000),
            |body| body.contains("thread_count") && !body.contains("\"thread_count\":0"),
        );

        let threads_response: JsonThreadsList =
            serde_json::from_str(&json_text).expect("Failed to parse threads JSON");

        assert!(
            threads_response.thread_count > 0,
            "Expected at least 1 thread, got {}",
            threads_response.thread_count
        );

        assert_eq!(
            threads_response.thread_count,
            threads_response.data.len(),
            "thread_count should match data.len()"
        );

        let hp_threads: Vec<_> = threads_response
            .data
            .iter()
            .filter(|t| t.name.starts_with("hp-"))
            .collect();

        assert!(
            !hp_threads.is_empty(),
            "Expected at least one hp- thread, found none. Threads: {:?}",
            threads_response
                .data
                .iter()
                .map(|t| &t.name)
                .collect::<Vec<_>>()
        );

        for thread in &threads_response.data {
            assert!(thread.os_tid > 0, "Thread should have valid os_tid");
        }
    }

    // cargo run -p test-tokio-async --example guard_timeout_threads --features hotpath
    #[test]
    fn test_guard_timeout_output() {
        let stdout = Example::new("test-tokio-async", "guard_timeout_threads").stdout();

        assert_contains_all(
            &stdout,
            &[
                "[hotpath]",
                "| threads",
                "Thread CPU and memory statistics.",
                "Max RSS:",
            ],
        );
    }

    // HOTPATH_OUTPUT_FORMAT=json cargo run -p test-tokio-async --example rss_exec --features hotpath
    #[cfg(target_os = "linux")]
    #[test]
    fn test_max_rss_excludes_pre_exec_peak() {
        let report = Example::new("test-tokio-async", "rss_exec").json().report();
        let threads = report.threads.expect("threads section");
        let max_rss = threads.rss_bytes_max.as_deref().expect("rss_bytes_max");
        let max_rss = hotpath::parse_bytes(max_rss).expect("rss_bytes_max parses");

        // The example touches 200 MB before exec'ing into the profiled run.
        assert!(
            max_rss < 100 * 1024 * 1024,
            "Max RSS {max_rss} B includes the pre-exec ballast"
        );
    }
}
