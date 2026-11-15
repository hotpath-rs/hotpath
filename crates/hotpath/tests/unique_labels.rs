#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
mod tests {
    use std::process::Command;

    use crate::common::example::Example;

    /// Builds one of the `duplicate_labels_*` fixture examples with its body
    /// enabled and returns rustc's stderr; the fixture is expected to fail
    /// codegen. `cargo rustc` hands the `--cfg` only to the example crate, so
    /// hotpath and its dependencies keep their fingerprints.
    fn build_fixture(example: &str) -> String {
        let output = Command::new("cargo")
            .args([
                "rustc",
                "-p",
                "test-all-features",
                "--example",
                example,
                "--features",
                "hotpath",
                "--",
                "--cfg",
                "hotpath_dup_labels_fixture",
            ])
            .output()
            .expect("Failed to execute cargo rustc");
        assert!(
            !output.status.success(),
            "{example} built although it repeats a literal label"
        );
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    // cargo rustc -p test-all-features --example duplicate_labels_channel --features hotpath -- --cfg hotpath_dup_labels_fixture
    #[test]
    fn test_duplicate_channel_label_fails_build() {
        let stderr = build_fixture("duplicate_labels_channel");
        assert!(
            stderr.contains("symbol `__hotpath_unique_channel_shared_")
                && stderr.contains("is already defined"),
            "unexpected build error:\n{stderr}"
        );
        assert!(
            stderr.contains("examples/duplicate_labels_channel.rs"),
            "error should point at the fixture call site:\n{stderr}"
        );
    }

    // cargo rustc -p test-all-features --example duplicate_labels_function --features hotpath -- --cfg hotpath_dup_labels_fixture
    #[test]
    fn test_measure_and_measure_block_share_function_namespace() {
        let stderr = build_fixture("duplicate_labels_function");
        assert!(
            stderr.contains("symbol `__hotpath_unique_function_shared_")
                && stderr.contains("is already defined"),
            "unexpected build error:\n{stderr}"
        );
    }

    // cargo run -p test-all-features --example unique_labels --features hotpath
    #[test]
    fn test_same_label_across_kinds_is_allowed() {
        let report = Example::new("test-all-features", "unique_labels").report();

        // The literal arms hand the label on to the runtime unchanged.
        let channels = report.channels.expect("No channels section");
        let channel_labels: Vec<&str> = channels.data.iter().map(|e| e.label.as_str()).collect();
        assert!(channel_labels.contains(&"shared"), "{channel_labels:?}");
        assert!(
            channel_labels.contains(&"shared-runtime"),
            "{channel_labels:?}"
        );

        let streams = report.streams.expect("No streams section");
        assert!(streams.data.iter().any(|e| e.label == "shared"));
        let futures = report.futures.expect("No futures section");
        assert!(futures.data.iter().any(|e| e.label == "shared"));
        let mutexes = report.mutexes.expect("No mutexes section");
        assert!(mutexes.data.iter().any(|e| e.label == "shared"));
        let rw_locks = report.rw_locks.expect("No rw_locks section");
        assert!(rw_locks.data.iter().any(|e| e.label == "shared"));
        let io = report.io.expect("No io section");
        assert!(io.data.iter().any(|e| e.label == "shared"));

        let functions = report.functions_timing.expect("No functions section");
        let names: Vec<&str> = functions.data.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"shared"), "{names:?}");
        assert!(names.contains(&"shared-block"), "{names:?}");
    }

    // cargo run -p test-all-features --example unique_labels --features hotpath
    #[test]
    fn test_labeled_marks_user_named_entries() {
        let report = Example::new("test-all-features", "unique_labels").report();

        // Literal and runtime labels alike; the guard's own entry and a
        // plain `#[measure]` are named by hotpath.
        let functions = report.functions_timing.expect("No functions section");
        let labeled: Vec<(&str, bool)> = functions
            .data
            .iter()
            .map(|e| (e.name.as_str(), e.labeled))
            .collect();
        for (name, expected) in [
            ("shared", true),
            ("shared-block", true),
            ("shared-block-runtime", true),
            ("unique_labels::unlabeled", false),
            ("main", false),
        ] {
            assert!(labeled.contains(&(name, expected)), "{name}: {labeled:?}");
        }

        let channels = report.channels.expect("No channels section");
        let channels: Vec<(&str, bool)> = channels
            .data
            .iter()
            .map(|e| (e.label.as_str(), e.labeled))
            .collect();
        assert!(channels.contains(&("shared", true)), "{channels:?}");
        assert!(channels.contains(&("shared-runtime", true)), "{channels:?}");
        assert!(
            channels.iter().any(|&(_, labeled)| !labeled),
            "{channels:?}"
        );

        let streams = report.streams.expect("No streams section");
        assert!(streams.data.iter().all(|e| e.labeled), "{:?}", streams.data);
        let futures = report.futures.expect("No futures section");
        assert!(futures.data.iter().all(|e| e.labeled));
        let mutexes = report.mutexes.expect("No mutexes section");
        assert!(mutexes.data.iter().all(|e| e.labeled));
        let rw_locks = report.rw_locks.expect("No rw_locks section");
        assert!(rw_locks.data.iter().all(|e| e.labeled));
        let io = report.io.expect("No io section");
        assert!(io.data.iter().all(|e| e.labeled));
    }

    #[test]
    fn test_labeled_defaults_to_false_when_absent() {
        let entry: hotpath::json::JsonFunctionEntry = serde_json::from_value(serde_json::json!({
            "name": "f", "calls": 1, "sampled_calls": 1, "avg": "1 ns", "total": "1 ns",
            "percent_total": "100.00%"
        }))
        .unwrap();
        assert!(!entry.labeled);
    }
}
