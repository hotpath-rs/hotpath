#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use std::process::Command;

    use hotpath::json::JsonStreamsList;

    use crate::common::assert_contains_all;
    use crate::common::endpoints::assert_list_and_logs_endpoints;
    use crate::common::example::Example;

    fn example(name: &str) -> Example {
        Example::new("test-streams", name)
    }

    fn streams(name: &str) -> JsonStreamsList {
        example(name)
            .report()
            .streams
            .expect("No streams section in report")
    }

    // cargo run -p test-streams --example agg_streams --features hotpath
    #[test]
    fn test_default_mode_aggregates_per_callsite() {
        let streams = streams("agg_streams");

        // Default mode: 4 loop-created streams collapse into one entry.
        let agg = streams
            .data
            .iter()
            .find(|s| !s.has_custom_label)
            .expect("aggregated entry not found");
        assert_eq!(agg.instances, 4, "4 streams created at the call site");
        assert_eq!(agg.closed_instances, 4, "all streams completed");
        assert_eq!(agg.state, None, "aggregated entries report no state");
        assert_eq!(agg.items_yielded, 20, "summed across instances");
        assert_eq!(agg.iter, 0, "aggregated entries carry no iter suffix");

        // iter = true: one suffixed entry per instance.
        for label in ["itered", "itered-2", "itered-3"] {
            let entry = streams
                .data
                .iter()
                .find(|s| s.label == label)
                .unwrap_or_else(|| panic!("per-instance entry {label} not found"));
            assert_eq!(entry.instances, 1);
            assert_eq!(entry.items_yielded, 2);
        }
        assert_eq!(streams.data.len(), 4, "one aggregated + three per-instance");
    }

    // cargo build -p test-streams --example agg_streams
    #[test]
    fn test_iter_param_compiles_without_feature() {
        let output = Command::new("cargo")
            .args(["build", "-p", "test-streams", "--example", "agg_streams"])
            .output()
            .expect("Failed to execute command");

        assert!(
            output.status.success(),
            "feature-off build of `stream!(..., iter = true)` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // cargo run -p test-streams --example basic_streams --features hotpath
    #[test]
    fn test_basic_streams_output() {
        let stdout = example("basic_streams").stdout();

        let all_expected = [
            "number-stream",
            "text-stream",
            "repeat-stream",
            "Stream example completed!",
            "Stream yield statistics",
            "5", // number-stream yielded 5 items
            "4", // text-stream yielded 4 items
            "3", // repeat-stream yielded 3 items
            "Yielded",
        ];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-streams --example basic_streams --features hotpath
    #[test]
    fn test_streams_closed_state() {
        let stdout = example("basic_streams").stdout();

        let closed_count = stdout.matches("| closed").count();
        assert!(
            closed_count >= 3,
            "Expected at least 3 'closed' states for streams, found {}.\nOutput:\n{}",
            closed_count,
            stdout
        );
    }

    // HOTPATH_METRICS_PORT=6774 TEST_SLEEP_SECONDS=10 cargo run -p test-streams --example basic_streams --features hotpath
    #[test]
    fn test_data_endpoints() {
        assert_list_and_logs_endpoints(
            example("basic_streams"),
            6774,
            "streams",
            &["basic_streams.rs", "number-stream", "text-stream"],
            |streams: &JsonStreamsList| streams.data.first().and_then(|stream| stream.id),
        );
    }

    // cargo run -p test-streams --example guard_timeout_streams --features hotpath
    #[test]
    fn test_guard_timeout_output() {
        let stdout = example("guard_timeout_streams").stdout();
        let expected_content = ["[hotpath]", "| streams", "timeout-stream"];

        assert_contains_all(&stdout, &expected_content);
    }

    // HOTPATH_OUTPUT_FORMAT=none cargo run -p test-streams --example basic_streams --features hotpath
    #[test]
    fn test_format_none_suppresses_output() {
        let stdout = example("basic_streams")
            .env("HOTPATH_OUTPUT_FORMAT", "none")
            .stdout();

        assert!(
            stdout.contains("Stream example completed!"),
            "Application output should still be present.\nGot:\n{stdout}"
        );

        let not_expected = [
            "[hotpath]",
            "number-stream",
            "text-stream",
            "Stream yield statistics",
        ];

        for not_exp in not_expected {
            assert!(
                !stdout.contains(not_exp),
                "Stream output should be suppressed with HOTPATH_OUTPUT_FORMAT=none.\nFound: {not_exp}\nGot:\n{stdout}"
            );
        }
    }

    // cargo run -p test-streams --example streams_file_output --features hotpath
    #[test]
    fn test_streams_file_output() {
        use std::fs;
        use std::path::Path;

        let output_path = "tmp/streams_output_test.json";

        fs::create_dir_all("tmp").ok();
        if Path::new(output_path).exists() {
            fs::remove_file(output_path).ok();
        }

        example("streams_file_output").stdout();

        assert!(
            Path::new(output_path).exists(),
            "Output file was not created at {}",
            output_path
        );

        let file_content = fs::read_to_string(output_path).expect("Failed to read output file");

        let expected_content = ["number-stream", "\"items_yielded\""];

        assert_contains_all(&file_content, &expected_content);

        fs::remove_file(output_path).ok();
    }

    // Two `stream!` invocations on one physical line (same item type) must
    // register distinct entries: the registration key includes the column, so
    // the second call site does not reuse the first one's id. The displayed
    // source keeps the plain `file:line` form.
    //
    // cargo run -p test-streams --example same_line_streams --features hotpath
    #[test]
    fn test_same_line_call_sites_stay_distinct() {
        let streams = streams("same_line_streams");

        let a = streams
            .data
            .iter()
            .find(|s| s.label == "same-line-a")
            .expect("same-line-a stream not found");
        let b = streams
            .data
            .iter()
            .find(|s| s.label == "same-line-b")
            .expect("same-line-b stream not found");

        assert_ne!(a.id, b.id, "same-line call sites must not share an entry");
        assert_eq!(
            a.items_yielded, 3,
            "counts must not merge across call sites"
        );
        assert_eq!(
            b.items_yielded, 7,
            "counts must not merge across call sites"
        );
        assert_eq!(a.instances, 1);
        assert_eq!(b.instances, 1);
        assert_eq!(a.closed_instances, 1);
        assert_eq!(b.closed_instances, 1);

        // The displayed source is identical for both (same file:line) and the
        // path contains no ':', so exactly one colon proves the column stays
        // out of the display string.
        assert_eq!(a.source, b.source, "one physical line renders one source");
        assert_eq!(
            a.source.matches(':').count(),
            1,
            "source must stay file:line"
        );
    }
}
