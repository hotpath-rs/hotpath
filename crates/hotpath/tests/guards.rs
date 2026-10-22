#[path = "common/support.rs"]
mod common;

#[cfg(test)]
pub mod tests {
    use serde_json::Value;

    use crate::common::example::Example;

    fn example(features: &str) -> Example {
        Example::new("test-smol-async", "all_guards").features(features)
    }

    fn run_all_guards_json(features: &str, report: &str) -> Value {
        let stdout = example(features)
            .json()
            .env("HOTPATH_REPORT", report)
            .env("HOTPATH_METRICS_SERVER_OFF", "true")
            .stdout();

        serde_json::from_str(stdout.lines().last().expect("No JSON output line"))
            .expect("Failed to parse JSON report")
    }

    fn assert_functions_present(report: &Value) {
        let functions = report["functions_timing"]["data"]
            .as_array()
            .expect("Expected functions_timing.data array");

        let expected = [
            "all_guards::sync_plain",
            "all_guards::sync_log",
            "all_guards::async_plain",
            "all_guards::async_log",
            "all_guards::async_future",
            "all_guards::async_future_log",
        ];

        for name in expected {
            let entry = functions
                .iter()
                .find(|f| f["name"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("Expected {name} in functions_timing"));
            assert_eq!(
                entry["calls"].as_u64(),
                Some(3),
                "Expected 3 calls for {name}"
            );
        }
    }

    fn assert_futures_present(report: &Value) {
        let futures = report["futures"]["data"]
            .as_array()
            .expect("Expected futures.data array");

        let with_future = ["all_guards::async_future", "all_guards::async_future_log"];
        for source in with_future {
            let entry = futures
                .iter()
                .find(|f| f["source"].as_str() == Some(source))
                .unwrap_or_else(|| panic!("Expected future entry for {source}"));
            assert_eq!(
                entry["call_count"].as_u64(),
                Some(3),
                "Expected 3 calls for {source}"
            );
        }

        let without_future = ["all_guards::async_plain", "all_guards::async_log"];
        for source in without_future {
            assert!(
                futures.iter().all(|f| f["source"].as_str() != Some(source)),
                "Did not expect future entry for {source}"
            );
        }
    }

    // HOTPATH_OUTPUT_FORMAT=none cargo run -p test-smol-async --example all_guards --features hotpath-alloc
    #[test]
    fn test_measure_impl_guards_example_runs() {
        example("hotpath-alloc")
            .env("HOTPATH_OUTPUT_FORMAT", "none")
            .stdout();
    }

    // HOTPATH_OUTPUT_FORMAT=json HOTPATH_REPORT=functions-timing,futures cargo run -p test-smol-async --example all_guards --features hotpath
    #[test]
    fn test_measure_impl_guards_json_hotpath() {
        let report = run_all_guards_json("hotpath", "functions-timing,futures");
        assert_functions_present(&report);
        assert_futures_present(&report);
    }

    // HOTPATH_OUTPUT_FORMAT=json HOTPATH_REPORT=functions-timing,functions-alloc,futures cargo run -p test-smol-async --example all_guards --features hotpath-alloc
    #[test]
    fn test_measure_impl_guards_json_hotpath_alloc() {
        let report =
            run_all_guards_json("hotpath-alloc", "functions-timing,functions-alloc,futures");

        assert_functions_present(&report);
        assert_futures_present(&report);

        let functions_alloc = report["functions_alloc"]["data"]
            .as_array()
            .expect("Expected functions_alloc.data array");
        let expected = [
            "all_guards::sync_plain",
            "all_guards::sync_log",
            "all_guards::async_plain",
            "all_guards::async_log",
            "all_guards::async_future",
            "all_guards::async_future_log",
        ];
        for name in expected {
            functions_alloc
                .iter()
                .find(|f| f["name"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("Expected {name} in functions_alloc"));
        }
    }
}
