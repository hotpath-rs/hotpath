#[path = "common/support.rs"]
mod common;

#[cfg(test)]
pub mod tests {
    use std::process::Command;

    use crate::common::assert_contains_all;
    use crate::common::example::Example;

    fn example(name: &str) -> Example {
        Example::new("test-tokio-async", name).env("HOTPATH_REPORT", "functions-timing")
    }

    // cargo run -p test-tokio-async --example basic --features hotpath
    #[test]
    fn test_basic_output() {
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

    // cargo run -p test-tokio-async --example early_returns --features hotpath
    #[test]
    fn test_early_returns_output() {
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

    // cargo run -p test-tokio-async --example main_empty --features hotpath
    #[test]
    fn test_main_empty_params() {
        let stdout = example("main_empty").stdout();

        assert_contains_all(
            &stdout,
            &["main_empty::example_function", "main_empty::main"],
        );
    }

    // cargo run -p test-tokio-async --example main_attrs_below --features hotpath
    #[test]
    fn test_main_keeps_attrs_below() {
        let stdout = example("main_attrs_below").json().stdout();

        assert_contains_all(&stdout, &["main_attrs_below::example_function"]);
    }

    // cargo run -p test-tokio-async --example main_percentiles --features hotpath
    #[test]
    fn test_main_percentiles_param() {
        let stdout = example("main_percentiles").stdout();

        assert_contains_all(
            &stdout,
            &[
                "main_percentiles::example_function",
                "P50",
                "P90",
                "P99",
                "Function",
            ],
        );
    }

    // cargo run -p test-tokio-async --example main_format --features hotpath
    #[test]
    fn test_main_format_param() {
        let stdout = example("main_format").stdout();

        assert_contains_all(
            &stdout,
            &[
                "main_format::example_function",
                "\"profiling_mode\"",
                "\"calls\"",
            ],
        );
    }

    // cargo run -p test-tokio-async --example main_percentiles_format --features hotpath
    #[test]
    fn test_main_percentiles_format_params() {
        let stdout = example("main_percentiles_format").stdout();

        assert_contains_all(
            &stdout,
            &[
                "main_percentiles_format::example_function",
                "\"profiling_mode\"",
                "\"p75\"",
                "\"p95\"",
            ],
        );
    }

    // cargo run -p test-all-features --example basic_all_features --all-features
    // hotpath-cpu is macOS/Linux only, so --all-features cannot build on Windows.
    #[cfg(not(windows))]
    #[test]
    fn test_all_features_output() {
        let output = Command::new("cargo")
            .args([
                "run",
                "-p",
                "test-all-features",
                "--example",
                "basic_all_features",
                "--all-features",
            ])
            .env("HOTPATH_REPORT", "functions-timing")
            .output()
            .expect("Failed to execute command");

        assert!(
            output.status.success(),
            "Process did not exit successfully.\n\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);

        assert_contains_all(&stdout, &["i ran"]);
    }

    // cargo run -p test-tokio-async --example no_op_block
    #[test]
    fn test_no_op_block_output() {
        let output = Command::new("cargo")
            .args(["run", "-p", "test-tokio-async", "--example", "no_op_block"])
            .env("HOTPATH_REPORT", "functions-timing")
            .output()
            .expect("Failed to execute command");

        assert!(
            output.status.success(),
            "Process did not exit successfully.\n\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("custom_block output"));
    }

    // cargo run -p test-tokio-async --example custom_guard --features hotpath
    #[test]
    fn test_custom_guard_output() {
        let output = example("custom_guard").output();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_contains_all(
            &stdout,
            &[
                "custom_guard::main",
                "custom_guard::sync_function",
                "custom_guard::async_function",
                "custom_block",
            ],
        );
    }

    // cargo run -p test-tokio-async --example measure_all_mod --features hotpath
    #[test]
    fn test_measure_all_mod_output() {
        let stdout = example("measure_all_mod").stdout();

        assert_contains_all(
            &stdout,
            &[
                "measured_module::sync_function_one",
                "measured_module::async_function_one",
                "measure_all_mod::main",
                "| measured_module::async_function_one | 50    |",
            ],
        );

        let not_expected_content = [
            "measured_module::sync_function_two",
            "measured_module::async_function_two",
        ];

        for not_expected in not_expected_content {
            assert!(
                !stdout.contains(not_expected),
                "Not expected:\n{not_expected}\n\nGot:\n{stdout}",
            );
        }
    }

    // cargo run -p test-tokio-async --example measure_all_impl --features hotpath
    #[test]
    fn test_measure_all_impl_output() {
        let stdout = example("measure_all_impl").stdout();

        assert_contains_all(
            &stdout,
            &[
                "Calculator::new",
                "measure_all_impl::add",
                "Calculator::multiply",
                "Calculator::async_increment",
                "Calculator::async_decrement",
                "Calculator::get_value",
                "measure_all_impl::main",
                "measure_all_impl::add",
            ],
        );
    }

    // cargo check -p test-tokio-async --example measure_all_impl_return_closure --features hotpath
    #[test]
    fn test_measure_all_impl_return_closure_compiles() {
        let output = Command::new("cargo")
            .args([
                "check",
                "-p",
                "test-tokio-async",
                "--example",
                "measure_all_impl_return_closure",
                "--features",
                "hotpath",
            ])
            .env("HOTPATH_REPORT", "functions-timing")
            .output()
            .expect("Failed to execute command");

        assert!(
            output.status.success(),
            "Process did not exit successfully.\n\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // cargo run -p test-tokio-async --example main_timeout --features hotpath
    #[test]
    fn test_main_timeout_output() {
        let stdout = example("main_timeout")
            .env("HOTPATH_SHUTDOWN_MS", "1000")
            .stdout();

        assert_contains_all(
            &stdout,
            &[
                "main_timeout::first_function",
                "main_timeout::second_function",
                "loop_block",
                "main_timeout::main",
            ],
        );
    }

    // cargo run -p test-tokio-async --example guard_timeout --features hotpath
    #[test]
    fn test_guard_timeout_output() {
        let stdout = example("guard_timeout").stdout();

        assert_contains_all(
            &stdout,
            &[
                "guard_timeout::first_function",
                "guard_timeout::second_function",
                "loop_block",
            ],
        );
    }

    // cargo run -p test-tokio-async --example guard_timeout_functions --features hotpath
    #[test]
    fn test_guard_timeout_functions_output() {
        let stdout = example("guard_timeout_functions").stdout();

        assert_contains_all(&stdout, &["guard_timeout_functions::looping_function"]);
    }

    // HOTPATH_EXCLUDE_WRAPPER=1 cargo run -p test-tokio-async --example basic --features hotpath
    #[test]
    fn test_exclude_wrapper_output() {
        let stdout = example("basic")
            .env("HOTPATH_EXCLUDE_WRAPPER", "1")
            .stdout();

        assert_contains_all(
            &stdout,
            &[
                "basic::sync_function",
                "basic::async_function",
                "custom_block",
            ],
        );

        assert!(
            !stdout.contains("\"name\":\"basic::main\""),
            "Wrapper function 'basic::main' should not be in data array when HOTPATH_EXCLUDE_WRAPPER=1\n\nGot:\n{stdout}"
        );
    }

    // HOTPATH_METRICS_PORT=6776 HOTPATH_METRICS_SERVER_OFF=true TEST_SLEEP_SECONDS=5 cargo run -p test-tokio-async --example basic --features hotpath
    #[test]
    fn test_disable_http_server() {
        use std::{thread::sleep, time::Duration};

        let _running = example("basic")
            .env("HOTPATH_METRICS_PORT", "6776")
            .env("HOTPATH_METRICS_SERVER_OFF", "true")
            .env("TEST_SLEEP_SECONDS", "5")
            .spawn();

        sleep(Duration::from_secs(2));

        let result = ureq::get("http://127.0.0.1:6776/functions_timing").call();

        assert!(
            result.is_err(),
            "HTTP request should have failed when HOTPATH_METRICS_SERVER_OFF=true"
        );
    }

    // cargo run -p test-tokio-async --example functions_file_output --features hotpath
    #[test]
    fn test_functions_file_output() {
        use std::fs;
        use std::path::Path;

        let output_path = "tmp/functions_output_test.json";

        fs::create_dir_all("tmp").ok();
        if Path::new(output_path).exists() {
            fs::remove_file(output_path).ok();
        }

        example("functions_file_output").stdout();

        assert!(
            Path::new(output_path).exists(),
            "Output file was not created at {}",
            output_path
        );

        let file_content = fs::read_to_string(output_path).expect("Failed to read output file");

        assert_contains_all(
            &file_content,
            &[
                "functions_file_output::example_function",
                "\"profiling_mode\"",
                "\"calls\"",
            ],
        );

        fs::remove_file(output_path).ok();
    }

    // HOTPATH_OUTPUT_FORMAT=none cargo run -p test-tokio-async --example basic --features hotpath
    #[test]
    fn test_format_none_suppresses_output() {
        let stdout = example("basic")
            .env("HOTPATH_OUTPUT_FORMAT", "none")
            .stdout();

        assert!(
            stdout.contains("custom_block output"),
            "Application output should still be present.\nGot:\n{stdout}"
        );

        let not_expected = [
            "basic::sync_function",
            "basic::async_function",
            "profiling_mode",
            "percent_total",
        ];

        for not_exp in not_expected {
            assert!(
                !stdout.contains(not_exp),
                "Profiling output should be suppressed with HOTPATH_OUTPUT_FORMAT=none.\nFound: {not_exp}\nGot:\n{stdout}"
            );
        }
    }

    // HOTPATH_OUTPUT_PATH=tmp/env_override.json cargo run -p test-tokio-async --example functions_file_output --features hotpath
    #[test]
    fn test_hotpath_output_path_env_override() {
        use std::fs;
        use std::path::Path;

        let programmatic_path = "tmp/functions_output_test.json";
        let env_override_path = "tmp/env_override.json";

        fs::create_dir_all("tmp").ok();
        if Path::new(programmatic_path).exists() {
            fs::remove_file(programmatic_path).ok();
        }
        if Path::new(env_override_path).exists() {
            fs::remove_file(env_override_path).ok();
        }

        example("functions_file_output")
            .env("HOTPATH_OUTPUT_PATH", env_override_path)
            .stdout();

        assert!(
            Path::new(env_override_path).exists(),
            "Output file was not created at env override path {}",
            env_override_path
        );

        assert!(
            !Path::new(programmatic_path).exists(),
            "Output file should NOT be created at programmatic path {} when HOTPATH_OUTPUT_PATH is set",
            programmatic_path
        );

        fs::remove_file(env_override_path).ok();
    }

    // HOTPATH_OUTPUT_FORMAT=table HOTPATH_FOCUS=basic cargo run -p test-tokio-async --example basic --features hotpath
    #[test]
    fn test_focus_substring_filter() {
        let stdout = example("basic")
            .env("HOTPATH_OUTPUT_FORMAT", "table")
            .env("HOTPATH_FOCUS", "basic")
            .stdout();

        assert_contains_all(
            &stdout,
            &[
                "basic::sync_function",
                "basic::async_function",
                "basic::main",
            ],
        );

        assert!(
            !stdout.contains("| custom_block"),
            "custom_block should be filtered out by HOTPATH_FOCUS=basic\n\nGot:\n{stdout}"
        );
    }

    // HOTPATH_OUTPUT_FORMAT=table HOTPATH_FOCUS='/(custom)/' cargo run -p test-tokio-async --example basic --features hotpath
    #[test]
    fn test_focus_regex_filter() {
        let stdout = example("basic")
            .env("HOTPATH_OUTPUT_FORMAT", "table")
            .env("HOTPATH_FOCUS", "/(custom)/")
            .stdout();

        assert!(
            stdout.contains("| custom_block"),
            "Expected custom_block in profiling output\n\nGot:\n{stdout}",
        );

        assert!(
            stdout.contains("basic::main"),
            "Wrapper function basic::main should never be excluded by HOTPATH_FOCUS\n\nGot:\n{stdout}",
        );

        let not_expected = ["| basic::sync_function", "| basic::async_function"];

        for not_exp in not_expected {
            assert!(
                !stdout.contains(not_exp),
                "{not_exp} should be filtered out by HOTPATH_FOCUS=/(custom)/\n\nGot:\n{stdout}"
            );
        }
    }

    // cargo run -p test-tokio-async --example measure_label --features hotpath
    #[test]
    fn test_measure_label_output() {
        let stdout = example("measure_label").stdout();

        assert_contains_all(&stdout, &["| sync_labeled", "| async_labeled"]);

        let not_expected = [
            "measure_label::sync_function",
            "measure_label::async_function",
            "measure_label::sync_labeled",
            "measure_label::async_labeled",
        ];

        for not_exp in not_expected {
            assert!(
                !stdout.contains(not_exp),
                "Function name should be replaced by label. Found: {not_exp}\n\nGot:\n{stdout}"
            );
        }
    }

    // cargo run -p test-tokio-async --example early_returns --features hotpath
    #[test]
    fn test_report_footer_hint() {
        const HINT: &str = "Prevent performance regressions in CI";
        let run = |env: &[(&str, &str)]| {
            example("early_returns")
                .env_remove("HOTPATH_DISABLE_HINTS")
                .env_remove("HOTPATH_API_TOKEN")
                .env_remove("HOTPATH_OUTPUT_FORMAT")
                .env_remove("HOTPATH_OUTPUT_PATH")
                .env_remove("HOTPATH_UPLOAD")
                .envs(env)
                .stdout()
        };

        let stdout = run(&[]);
        assert!(stdout.contains(HINT), "Expected hint\n\nGot:\n{stdout}");

        for env in [
            ("HOTPATH_DISABLE_HINTS", "1"),
            ("HOTPATH_API_TOKEN", "token"),
            ("HOTPATH_OUTPUT_FORMAT", "json"),
        ] {
            let stdout = run(&[env]);
            assert!(
                !stdout.contains(HINT),
                "Unexpected hint with {env:?}\n\nGot:\n{stdout}"
            );
        }
    }

    const EXCLUSIVE_FEATURE_SETS: [&str; 2] = ["hotpath", "hotpath,hotpath-alloc"];

    /// Function timing rows of the `exclusive_timing` example, plus stderr.
    fn run_exclusive_timing(
        features: &str,
        env: &[(&str, &str)],
    ) -> (hotpath::json::JsonFunctionsList, String) {
        let output = Command::new("cargo")
            .args([
                "run",
                "-p",
                "test-tokio-async",
                "--example",
                "exclusive_timing",
                "--features",
                features,
            ])
            .env("HOTPATH_REPORT", "functions-timing")
            .envs(env.iter().copied())
            .output()
            .expect("Failed to execute command");

        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            output.status.success(),
            "Process did not exit successfully.\n\nstderr:\n{stderr}",
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json_start = stdout.find('{').expect("No JSON report in output");
        let report: hotpath::json::JsonReport =
            serde_json::Deserializer::from_str(&stdout[json_start..])
                .into_iter::<hotpath::json::JsonReport>()
                .next()
                .expect("No JSON value in output")
                .expect("Failed to parse JSON report");
        let functions = report
            .functions_timing
            .expect("No functions_timing section in report");
        (functions, stderr)
    }

    /// Total time of `exclusive_timing::<name>` in milliseconds.
    fn total_ms(functions: &hotpath::json::JsonFunctionsList, name: &str) -> f64 {
        let full_name = format!("exclusive_timing::{name}");
        let entry = functions
            .data
            .iter()
            .find(|entry| entry.name == full_name)
            .unwrap_or_else(|| panic!("{full_name} missing in {functions:?}"));
        let total = hotpath::parse_duration(&entry.total)
            .unwrap_or_else(|| panic!("{full_name} total {:?} does not parse", entry.total));
        total as f64 / 1e6
    }

    // cargo run -p test-tokio-async --example exclusive_timing --features hotpath
    #[test]
    fn test_inclusive_timing_includes_nested_calls() {
        for features in EXCLUSIVE_FEATURE_SETS {
            let (functions, _) = run_exclusive_timing(features, &[]);
            let ms = |name| total_ms(&functions, name);

            assert!(ms("sync_parent") >= 150.0, "{features}: {functions:?}");
            assert!(ms("sync_parent") > ms("sync_child"), "{features}");
            // Every recursion level counts its nested levels again.
            assert!(ms("recursive") >= 200.0, "{features}: {functions:?}");
            assert!(ms("async_parent") >= 150.0, "{features}: {functions:?}");
            assert!(ms("async_sleeper") >= 100.0, "{features}: {functions:?}");
        }
    }

    // HOTPATH_TIME_EXCLUSIVE=true cargo run -p test-tokio-async --example exclusive_timing --features hotpath
    #[test]
    fn test_exclusive_timing_excludes_nested_calls() {
        for features in EXCLUSIVE_FEATURE_SETS {
            let (functions, _) =
                run_exclusive_timing(features, &[("HOTPATH_TIME_EXCLUSIVE", "true")]);
            let ms = |name| total_ms(&functions, name);

            assert!(
                functions.description.contains("Exclusive"),
                "{features}: {}",
                functions.description
            );

            assert!(ms("sync_parent") >= 50.0, "{features}: {functions:?}");
            assert!(ms("sync_child") >= 100.0, "{features}: {functions:?}");
            assert!(ms("sync_parent") < ms("sync_child"), "{features}");

            // 4 levels of 20ms each, no longer the inclusive 200ms.
            assert!(
                (80.0..160.0).contains(&ms("recursive")),
                "{features}: {functions:?}"
            );

            assert!(ms("focus_outer") < ms("focus_leaf"), "{features}");

            // Async exclusive time is busy time summed over polls: the nested
            // call is subtracted and time suspended at `.await` never counts.
            assert!(ms("async_parent") >= 50.0, "{features}: {functions:?}");
            assert!(ms("async_child") >= 100.0, "{features}: {functions:?}");
            assert!(ms("async_parent") < ms("async_child"), "{features}");
            assert!(ms("async_sleeper") < 50.0, "{features}: {functions:?}");

            let percent_sum: f64 = functions
                .data
                .iter()
                .map(|entry| {
                    entry
                        .percent_total
                        .trim_end_matches('%')
                        .parse::<f64>()
                        .unwrap_or_else(|_| panic!("{features}: {entry:?}"))
                })
                .sum();
            assert!(
                (percent_sum - 100.0).abs() < 1.0,
                "{features}: rows add up to {percent_sum}%: {functions:?}"
            );
        }
    }

    // HOTPATH_TIME_EXCLUSIVE=true HOTPATH_FOCUS='/focus_(outer|leaf)/' cargo run -p test-tokio-async --example exclusive_timing --features hotpath
    #[test]
    fn test_exclusive_timing_focus_skipped_flows_to_ancestor() {
        for features in EXCLUSIVE_FEATURE_SETS {
            let (functions, _) = run_exclusive_timing(
                features,
                &[
                    ("HOTPATH_TIME_EXCLUSIVE", "true"),
                    ("HOTPATH_FOCUS", "/focus_(outer|leaf)/"),
                ],
            );
            let ms = |name| total_ms(&functions, name);

            assert!(
                !functions
                    .data
                    .iter()
                    .any(|entry| entry.name.ends_with("focus_mid")),
                "{features}: {functions:?}"
            );
            // Own 30ms plus the 30ms of the filtered-out `focus_mid`; the
            // measured `focus_leaf` below it is still subtracted.
            assert!(
                (60.0..120.0).contains(&ms("focus_outer")),
                "{features}: {functions:?}"
            );
            assert!(ms("focus_leaf") >= 60.0, "{features}: {functions:?}");
        }
    }

    // HOTPATH_TIME_EXCLUSIVE=true HOTPATH_TIME_SAMPLING_RATE=0 cargo run -p test-tokio-async --example exclusive_timing --features hotpath
    #[test]
    fn test_exclusive_timing_overrides_functions_sampling() {
        for features in EXCLUSIVE_FEATURE_SETS {
            let (functions, stderr) = run_exclusive_timing(
                features,
                &[
                    ("HOTPATH_TIME_EXCLUSIVE", "true"),
                    ("HOTPATH_TIME_SAMPLING_RATE", "0"),
                ],
            );

            assert!(
                stderr.contains("HOTPATH_TIME_EXCLUSIVE needs every function call timed"),
                "{features}: {stderr}"
            );
            for entry in &functions.data {
                assert_eq!(entry.sampled_calls, entry.calls, "{features}: {entry:?}");
            }
            assert!(total_ms(&functions, "sync_child") >= 100.0, "{features}");
        }
    }
}
