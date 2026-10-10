#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use hotpath::json::{JsonFunctionsList, JsonReport};
    use hotpath::parse_duration;
    use std::process::Command;

    const EXCLUSIVE_FEATURE_SETS: [&str; 2] = ["hotpath", "hotpath,hotpath-alloc"];

    /// Function timing rows of the `exclusive_timing` example, plus stderr.
    fn run_exclusive_timing(features: &str, env: &[(&str, &str)]) -> (JsonFunctionsList, String) {
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
        let report: JsonReport = serde_json::Deserializer::from_str(&stdout[json_start..])
            .into_iter::<JsonReport>()
            .next()
            .expect("No JSON value in output")
            .expect("Failed to parse JSON report");
        let functions = report
            .functions_timing
            .expect("No functions_timing section in report");
        (functions, stderr)
    }

    /// Total time of `exclusive_timing::<name>` in milliseconds.
    fn total_ms(functions: &JsonFunctionsList, name: &str) -> f64 {
        let full_name = format!("exclusive_timing::{name}");
        let entry = functions
            .data
            .iter()
            .find(|entry| entry.name == full_name)
            .unwrap_or_else(|| panic!("{full_name} missing in {functions:?}"));
        let total = parse_duration(&entry.total)
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

            // 4 levels of 20ms each, below the inclusive 200ms.
            assert!(
                (80.0..200.0).contains(&ms("recursive")),
                "{features}: {functions:?}"
            );

            assert!(ms("focus_outer") < ms("focus_leaf"), "{features}");

            // Async exclusive time is busy time summed over polls: the nested
            // call is subtracted and time suspended at `.await` never counts.
            assert!(ms("async_parent") >= 50.0, "{features}: {functions:?}");
            assert!(ms("async_child") >= 100.0, "{features}: {functions:?}");
            assert!(ms("async_parent") < ms("async_child"), "{features}");
            assert!(ms("async_sleeper") < 50.0, "{features}: {functions:?}");

            // `% Total` is relative to the elapsed time. Everything here runs
            // on the main thread, where exclusive times (wrapper included)
            // partition that time.
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
                (90.0..=101.0).contains(&percent_sum),
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
