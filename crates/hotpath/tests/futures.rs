#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use hotpath::json::JsonFuturesList;

    use crate::common::endpoints::assert_list_and_logs_endpoints;
    use crate::common::example::Example;
    use crate::common::{assert_contains_all, path_sep};

    fn example(name: &str) -> Example {
        Example::new("test-futures", name)
    }

    // cargo run -p test-futures --example basic_futures --features hotpath
    #[test]
    fn test_basic_futures_output() {
        let stdout = example("basic_futures").stdout();

        let sep = path_sep();
        let futures_path = format!("| examples{sep}basic_futures.rs:");
        let all_expected = [
            "| labeled_with_log",
            "| my_labeled_future",
            "| basic_futures::attributed_no_log   | 2     | 4",
            "| basic_futures::attributed_with_log | 2     | 4",
            futures_path.as_str(),
        ];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-futures --example basic_futures --features hotpath
    #[test]
    fn test_futures_aggregation() {
        let stdout = example("basic_futures").stdout();

        assert!(
            stdout.contains("attributed_no_log"),
            "Expected 'attributed_no_log' function name in output.\nOutput:\n{}",
            stdout
        );

        assert!(
            stdout.contains("attributed_with_log"),
            "Expected 'attributed_with_log' function name in output.\nOutput:\n{}",
            stdout
        );

        assert!(
            stdout.contains("basic_futures.rs:"),
            "Expected 'basic_futures.rs:' file location in output.\nOutput:\n{}",
            stdout
        );

        // Each attributed fn: 2 calls x 2 polls = 4 polls.
        assert!(
            stdout.contains("| 2     | 4"),
            "Expected aggregated call count of 2 and poll count of 4.\nOutput:\n{}",
            stdout
        );
    }

    // HOTPATH_METRICS_PORT=6775 TEST_SLEEP_SECONDS=10 cargo run -p test-futures --example basic_futures --features hotpath
    #[test]
    fn test_data_endpoints() {
        let calls_text = assert_list_and_logs_endpoints(
            example("basic_futures"),
            6775,
            "futures",
            &["basic_futures.rs", "call_count", "total_polls"],
            |futures: &JsonFuturesList| futures.data.first().and_then(|future| future.id),
        );

        if let Some(calls_text) = calls_text {
            assert!(
                calls_text.contains("ready") || calls_text.contains("cancelled"),
                "Expected calls response to contain state info.\nGot:\n{}",
                calls_text
            );
        }
    }

    // cargo run -p test-futures --example guard_timeout_futures --features hotpath
    #[test]
    fn test_guard_timeout_output() {
        let stdout = example("guard_timeout_futures").stdout();
        let expected_content = [
            "[hotpath]",
            "| futures",
            "guard_timeout_futures::timeout_worker",
        ];

        assert_contains_all(&stdout, &expected_content);
    }

    // cargo run -p test-futures --example measure_future --features hotpath
    #[test]
    fn test_measure_future_output() {
        let stdout = example("measure_future").stdout();

        let expected_timing = [
            "measure_future::timed_future ",
            "measure_future::timed_future_with_log",
            "measure_future::timing_only",
        ];

        for expected in expected_timing {
            assert!(
                stdout.contains(expected),
                "Expected in timing table:\n{expected}\n\nGot:\n{stdout}",
            );
        }

        let expected_futures = [
            "| measure_future::timed_future          | 2     | 4",
            "| measure_future::timed_future_with_log | 1     | 2",
        ];

        for expected in expected_futures {
            assert!(
                stdout.contains(expected),
                "Expected in futures table:\n{expected}\n\nGot:\n{stdout}",
            );
        }

        if let Some(futures_section) = stdout.split("futures - Future poll").nth(1) {
            assert!(
                !futures_section.contains("timing_only"),
                "timing_only should not appear in futures table.\nFutures section:\n{futures_section}",
            );
        }
    }

    // HOTPATH_OUTPUT_FORMAT=none cargo run -p test-futures --example basic_futures --features hotpath
    #[test]
    fn test_format_none_suppresses_output() {
        let stdout = example("basic_futures")
            .env("HOTPATH_OUTPUT_FORMAT", "none")
            .stdout();

        let not_expected = [
            "[hotpath]",
            "Future poll",
            "attributed_no_log",
            "attributed_with_log",
        ];

        for not_exp in not_expected {
            assert!(
                !stdout.contains(not_exp),
                "Futures output should be suppressed with HOTPATH_OUTPUT_FORMAT=none.\nFound: {not_exp}\nGot:\n{stdout}"
            );
        }
    }

    // cargo run -p test-futures --example futures_file_output --features hotpath
    #[test]
    fn test_futures_file_output() {
        use std::fs;
        use std::path::Path;

        let output_path = "tmp/futures_output_test.json";

        fs::create_dir_all("tmp").ok();
        if Path::new(output_path).exists() {
            fs::remove_file(output_path).ok();
        }

        example("futures_file_output").stdout();

        assert!(
            Path::new(output_path).exists(),
            "Output file was not created at {}",
            output_path
        );

        let file_content = fs::read_to_string(output_path).expect("Failed to read output file");

        let expected_content = [
            "futures_file_output.rs:",
            "\"total_polls\"",
            "\"call_count\"",
        ];

        assert_contains_all(&file_content, &expected_content);

        fs::remove_file(output_path).ok();
    }
}
