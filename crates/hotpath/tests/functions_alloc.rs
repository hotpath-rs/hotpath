#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use crate::common::assert_contains_all;
    use crate::common::example::{poll_endpoint, Example};

    fn example_of(package: &str, name: &str) -> Example {
        Example::new(package, name)
            .features("hotpath-alloc")
            .env("HOTPATH_REPORT", "functions-alloc")
    }

    fn example(name: &str) -> Example {
        example_of("test-tokio-async", name)
    }

    // cargo run -p test-tokio-async --example basic --features hotpath-alloc
    #[test]
    fn test_basic_alloc_output() {
        for _ in 0..2 {
            let stdout = example("basic").stdout();

            assert_contains_all(
                &stdout,
                &[
                    "custom_block",
                    "basic::sync_function",
                    "basic::async_function",
                    "p95",
                    "total",
                    "percent_total",
                ],
            );
        }
    }

    // cargo run -p test-tokio-async --example early_returns --features hotpath-alloc
    #[test]
    fn test_early_returns_alloc_output() {
        for _ in 0..2 {
            let stdout = example("early_returns").stdout();

            assert_contains_all(
                &stdout,
                &[
                    "early_returns::early_return",
                    "early_returns::propagates_error",
                    "early_returns::normal_path",
                ],
            );
        }
    }

    // cargo run -p test-smol-async --example basic_smol --features hotpath-alloc -- --nocapture
    #[test]
    fn test_async_smol_alloc_profiling_output() {
        let stdout = example_of("test-smol-async", "basic_smol")
            .args(&["--nocapture"])
            .stdout();

        assert!(
            stdout.contains("basic_smol::main"),
            "Expected basic_smol::main in output\n\nGot:\n{stdout}",
        );
    }

    // cargo run -p test-tokio-async --example limit --features hotpath-alloc
    #[test]
    fn test_limit_output() {
        let stdout = example("limit").stdout();

        assert_contains_all(
            &stdout,
            &[
                "(3/4)",
                "limit::main",
                "measured_module::function_one",
                "measured_module::function_two",
            ],
        );

        let not_expected_content = ["limit::function_three", "N/A*"];

        for not_expected in not_expected_content {
            assert!(
                !stdout.contains(not_expected),
                "Not expected:\n{not_expected}\n\nGot:\n{stdout}"
            );
        }
    }

    // cargo run -p test-tokio-async --example multithread_alloc --features hotpath-alloc
    #[test]
    fn test_multithread_alloc_no_panic() {
        let test_cases = [
            ("hotpath-alloc", None),
            ("hotpath-alloc", None),
            ("hotpath-alloc", Some("true")),
            ("hotpath-alloc", Some("true")),
        ];

        for (features, alloc_cumulative) in test_cases {
            let mut run = example("multithread_alloc").features(features);

            if let Some(val) = alloc_cumulative {
                run = run.env("HOTPATH_ALLOC_CUMULATIVE", val);
            }

            let output = run.output();

            let env_info = alloc_cumulative
                .map(|v| format!("HOTPATH_ALLOC_CUMULATIVE={}", v))
                .unwrap_or_else(|| "no env var".to_string());

            assert!(
                output.status.success(),
                "Process did not exit successfully with features: {}, {}\n\nstderr:\n{}",
                features,
                env_info,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    // HOTPATH_METRICS_PORT=6775 TEST_SLEEP_SECONDS=10 cargo run -p test-tokio-async --example basic --features hotpath-alloc
    #[test]
    fn test_data_endpoints() {
        use hotpath::json::JsonFunctionsList;
        use std::time::Duration;

        let _running = example("basic")
            .env("HOTPATH_METRICS_PORT", "6775")
            .env("TEST_SLEEP_SECONDS", "10")
            .spawn();

        let timing_expected = [
            "basic::sync_function",
            "basic::async_function",
            "custom_block",
        ];

        let timing_json = poll_endpoint(
            "http://localhost:6775/functions_timing",
            18,
            Duration::from_millis(750),
            |body| timing_expected.iter().all(|e| body.contains(e)),
        );

        assert_contains_all(&timing_json, &timing_expected);

        let timing_response: JsonFunctionsList =
            serde_json::from_str(&timing_json).expect("Failed to parse timing JSON");

        let mut alloc_response = ureq::get("http://localhost:6775/functions_alloc")
            .call()
            .expect("Failed to call /functions_alloc endpoint");

        assert_eq!(
            alloc_response.status(),
            200,
            "Expected status 200 for /functions_alloc endpoint"
        );

        let alloc_json = alloc_response
            .body_mut()
            .read_to_string()
            .expect("Failed to read alloc response body");

        assert_contains_all(&alloc_json, &timing_expected);

        let _alloc_response: JsonFunctionsList =
            serde_json::from_str(&alloc_json).expect("Failed to parse alloc JSON");

        if let Some(first) = timing_response.data.first() {
            let function_id = first.id;

            let timing_logs_url = format!(
                "http://localhost:6775/functions_timing/{}/logs",
                function_id
            );
            let timing_logs_response = ureq::get(&timing_logs_url)
                .call()
                .expect("Failed to call /functions_timing/:id/logs endpoint");

            assert_eq!(
                timing_logs_response.status(),
                200,
                "Expected status 200 for /functions_timing/:id/logs endpoint"
            );

            let alloc_logs_url =
                format!("http://localhost:6775/functions_alloc/{}/logs", function_id);
            let alloc_logs_response = ureq::get(&alloc_logs_url)
                .call()
                .expect("Failed to call /functions_alloc/:id/logs endpoint");

            assert_eq!(
                alloc_logs_response.status(),
                200,
                "Expected status 200 for /functions_alloc/:id/logs endpoint"
            );
        }
    }

    // cargo run -p test-tokio-async --example basic --features hotpath-alloc
    #[test]
    fn test_alloc_total_bytes_not_inflated() {
        let report = example("basic").report();

        let alloc = report
            .functions_alloc
            .expect("Expected functions_alloc in report");

        let custom_block = alloc
            .data
            .iter()
            .find(|f| f.name == "custom_block")
            .expect("Expected custom_block in alloc data");

        assert_eq!(custom_block.calls, 100);

        let total_bytes =
            hotpath::parse_bytes(&custom_block.total).expect("Failed to parse custom_block total");
        assert!(
            total_bytes < 2048,
            "custom_block total should be under 2 KB, got {} B",
            total_bytes
        );
    }

    // cargo run -p test-tokio-async --example basic --features hotpath-alloc
    #[test]
    fn test_async_alloc_is_reported() {
        let report = example("basic")
            .env("HOTPATH_METRICS_SERVER_OFF", "true")
            .report();

        let alloc = report
            .functions_alloc
            .expect("Expected functions_alloc in report");

        let async_fn = alloc
            .data
            .iter()
            .find(|f| f.name == "basic::async_function")
            .expect("Expected basic::async_function in alloc data");

        assert_ne!(
            async_fn.total, "N/A",
            "async_function alloc should be reported when hotpath-alloc is enabled"
        );
    }

    // cargo run -p test-tokio-async --example alloc_measure --features hotpath-alloc
    #[test]
    fn test_alloc_uninstrumented_children_tracked() {
        let report = example("alloc_measure")
            .env("HOTPATH_ALLOC_CUMULATIVE", "true")
            .json()
            .env("HOTPATH_METRICS_SERVER_OFF", "true")
            .report();

        let alloc = report
            .functions_alloc
            .expect("Expected functions_alloc in report");

        let find = |name: &str| -> &hotpath::json::JsonFunctionEntry {
            alloc
                .data
                .iter()
                .find(|f| f.name.ends_with(&format!("::{name}")))
                .unwrap_or_else(|| panic!("Expected {name} in alloc data"))
        };

        let assert_bytes = |name: &str, expected: u64| {
            let entry = find(name);
            let bytes = hotpath::parse_bytes(&entry.total)
                .unwrap_or_else(|| panic!("Failed to parse total for {name}: {}", entry.total));
            assert_eq!(
                bytes, expected,
                "{name}: expected {expected} B, got {bytes} B"
            );
        };

        assert_bytes("uninstrumented_children_2kb", 2048);
        assert_bytes("own_1kb_plus_uninstrumented_child_1kb", 2048);
        assert_bytes(
            "own_1kb_plus_uninstrumented_1kb_plus_instrumented_1kb",
            3072,
        );
        assert_bytes("instrumented_1kb", 1024);
    }

    // cargo run -p test-tokio-async --example custom_allocator --features hotpath-alloc
    #[test]
    fn test_custom_allocator_via_main_macro() {
        let report = example("custom_allocator")
            .json()
            .env("HOTPATH_METRICS_SERVER_OFF", "true")
            .report();

        let alloc = report
            .functions_alloc
            .expect("Expected functions_alloc in report");

        let find = |name: &str| -> &hotpath::json::JsonFunctionEntry {
            alloc
                .data
                .iter()
                .find(|f| f.name == format!("custom_allocator::alloc_demo::{name}"))
                .unwrap_or_else(|| panic!("Expected {name} in alloc data"))
        };

        let alloc_work_bytes = hotpath::parse_bytes(&find("alloc_work").total)
            .expect("Failed to parse total bytes for alloc_work");
        assert!(
            alloc_work_bytes >= 1024,
            "expected alloc_work to report the zeroed 1 KB allocation, got {alloc_work_bytes} B"
        );

        // realloc_work allocates 512 B, then a realloc to >= 4608 B is
        // tracked as dealloc(512) + alloc(new_size).
        let realloc_work_bytes = hotpath::parse_bytes(&find("realloc_work").total)
            .expect("Failed to parse total bytes for realloc_work");
        assert!(
            realloc_work_bytes >= 512 + 4608,
            "expected realloc_work to count both the initial alloc and the realloc'd size, got {realloc_work_bytes} B"
        );
    }

    // cargo run -p test-alloc --example first_call_registration --features hotpath-alloc
    #[test]
    fn test_first_call_registration_reports_only_measured_allocs() {
        let report = example_of("test-alloc", "first_call_registration")
            .json()
            .env("HOTPATH_METRICS_SERVER_OFF", "true")
            .report();

        let alloc = report
            .functions_alloc
            .expect("Expected functions_alloc in report");

        let assert_total = |name: &str, expected: u64| {
            let entry = alloc
                .data
                .iter()
                .find(|f| f.name == format!("first_call_registration::{name}"))
                .unwrap_or_else(|| panic!("Expected {name} in alloc data"));
            assert_eq!(entry.calls, 10, "{name}: expected 10 calls");
            let bytes = hotpath::parse_bytes(&entry.total)
                .unwrap_or_else(|| panic!("Failed to parse total for {name}: {}", entry.total));
            assert_eq!(
                bytes, expected,
                "{name}: expected {expected} B, got {bytes} B"
            );
        };

        assert_total("outer", 500);
        assert_total("callee_a", 320);
        assert_total("callee_b", 0);
        assert_total("callee_c", 0);
        assert_total("callee_d", 0);
    }
}
