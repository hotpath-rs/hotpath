#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use std::process::Output;

    use hotpath::json::{JsonFunctionsList, JsonReport};

    use crate::common::callgrind::{valgrind_available, RunDir};
    use crate::common::example::{assert_success, Example};
    use crate::common::report::parse_report;

    const ITERATIONS: u64 = 1000;
    const ROUTINE: &str = "hotpath::criterion::routine";
    const BENCH: &str = "hotpath::criterion::bench";
    const WORK: &str = "fixture::work";

    #[derive(Debug, PartialEq)]
    struct Counts {
        routine_calls: u64,
        setup_calls: u64,
        input_drops: u64,
        output_drops: u64,
    }

    #[track_caller]
    fn counts(output: &Output) -> Counts {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = stderr
            .lines()
            .rev()
            .find_map(|l| l.strip_prefix("fixture: "))
            .unwrap_or_else(|| panic!("no fixture counts in stderr:\n{stderr}"));
        let field = |name: &str| -> u64 {
            line.split(' ')
                .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| panic!("no {name} in {line:?}"))
        };
        Counts {
            routine_calls: field("routine_calls"),
            setup_calls: field("setup_calls"),
            input_drops: field("input_drops"),
            output_drops: field("output_drops"),
        }
    }

    fn fixture(features: &str) -> Example {
        Example::new("test-criterion", "fixture")
            .features(features)
            .env_remove("HOTPATH_CRITERION_ITERATIONS")
    }

    fn fixed(features: &str, benchmark: &str) -> Example {
        fixture(features)
            .env("HOTPATH_CRITERION_ITERATIONS", ITERATIONS.to_string())
            .env("HOTPATH_FUNCTIONS_LIMIT", "0")
            .args(&["--exact", benchmark])
    }

    #[track_caller]
    fn entry<'a>(list: &'a JsonFunctionsList, name: &str) -> &'a hotpath::json::JsonFunctionEntry {
        list.data
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name} missing from {:#?}", list.data))
    }

    #[track_caller]
    fn bytes(value: &str) -> u64 {
        hotpath::parse_bytes(value).unwrap_or_else(|| panic!("not bytes: {value:?}"))
    }

    /// `hotpath-cloud` renders JSON values at exact precision.
    #[track_caller]
    fn fixed_run(benchmark: &str) -> (Counts, JsonReport) {
        let output = fixed("hotpath,hotpath-alloc,hotpath-cloud", benchmark)
            .json()
            .env_remove("HOTPATH_UPLOAD")
            .env_remove("GITHUB_ACTIONS")
            .env("HOTPATH_REPORT", "functions-timing,functions-alloc")
            .output();
        assert_success(&output);
        (
            counts(&output),
            parse_report(&String::from_utf8_lossy(&output.stdout)),
        )
    }

    // HOTPATH_CRITERION_ITERATIONS=1000 cargo run -p test-criterion --example fixture --features hotpath,hotpath-alloc -- --exact work/iter_batched_per_iteration
    #[test]
    fn test_fixed_iterations_run_each_routine_exactly_n_times() {
        let n = ITERATIONS;
        // (benchmark, setups, routine bytes per call, bench row bytes per
        // iteration, whether the bench row holds exactly those).
        let cases = [
            ("top_level", 0, 64 + 32, 0, true),
            ("top_input/9", 0, 64 + 32, 0, true),
            ("work/iter", 0, 64 + 32, 0, true),
            ("inputs/work/3", 0, 64 + 32, 0, true),
            ("inputs/seven", 0, 64 + 32, 0, true),
            (
                "work/iter_batched_per_iteration",
                n,
                64 + 16,
                1024 + 32,
                true,
            ),
            ("work/iter_batched", n, 64 + 16, 1024 + 32, false),
            (
                "work/iter_batched_ref_per_iteration",
                n,
                64,
                1024 + 32 + 16,
                true,
            ),
            ("work/iter_batched_ref", n, 64, 1024 + 32 + 16, true),
        ];
        for (benchmark, setups, routine_bytes, bench_bytes, exact) in cases {
            let (counts, report) = fixed_run(benchmark);
            assert_eq!(
                counts,
                Counts {
                    routine_calls: n,
                    setup_calls: setups,
                    input_drops: setups,
                    output_drops: n,
                },
                "{benchmark}"
            );
            let metadata = report.user_metadata.as_ref().expect("user metadata");
            assert_eq!(
                metadata.get("criterion_iterations").map(String::as_str),
                Some("1000")
            );
            assert_eq!(
                metadata.get("criterion_benchmark").map(String::as_str),
                Some(benchmark)
            );

            let timing = report.functions_timing.as_ref().expect("timing section");
            assert_eq!(timing.caller_name, BENCH);
            for name in [ROUTINE, WORK] {
                let e = entry(timing, name);
                assert_eq!((e.calls, e.sampled_calls), (n, n), "{benchmark} {name}");
            }
            assert_eq!(entry(timing, BENCH).calls, 1);

            let alloc = report.functions_alloc.as_ref().expect("alloc section");
            assert_eq!(bytes(&entry(alloc, WORK).avg), 512, "{benchmark}");
            assert_eq!(
                bytes(&entry(alloc, ROUTINE).avg),
                routine_bytes,
                "{benchmark}: setup and drops outside the routine must not be attributed to it"
            );
            let bench_total = bytes(&entry(alloc, BENCH).total);
            let expected = bench_bytes * n;
            if exact {
                assert_eq!(
                    bench_total, expected,
                    "{benchmark}: setup and drops outside the routine belong to the bench row"
                );
            } else {
                assert!(
                    bench_total > expected,
                    "{benchmark}: bench row {bench_total} B, expected over {expected} B"
                );
            }
        }
    }

    // HOTPATH_CRITERION_ITERATIONS=1000 cargo run -p test-criterion --example fixture --features hotpath,hotpath-alloc -- --exact work/iter_custom
    #[test]
    fn test_iter_custom_gets_the_iteration_count() {
        let (counts, report) = fixed_run("work/iter_custom");
        assert_eq!(counts.routine_calls, ITERATIONS);
        let timing = report.functions_timing.expect("timing section");
        assert_eq!(entry(&timing, WORK).calls, ITERATIONS);
        assert!(
            timing.data.iter().all(|e| e.name != ROUTINE),
            "iter_custom loops on its own, so it has no per-iteration row"
        );
    }

    // cargo run -p test-criterion --example fixture --features hotpath -- --exact work/iter_batched
    #[test]
    fn test_without_iterations_criterion_runs_unchanged() {
        // Test mode (implied by `cargo run`): one iteration, no report.
        let output = fixture("hotpath")
            .json()
            .args(&["--exact", "work/iter_batched"])
            .output();
        assert_success(&output);
        assert_eq!(
            counts(&output),
            Counts {
                routine_calls: 1,
                setup_calls: 1,
                input_drops: 1,
                output_drops: 1,
            }
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains('{'),
            "no hotpath report expected:\n{stdout}"
        );

        // Benchmark mode: Criterion's own warm-up and sampling loop.
        let output = fixture("hotpath")
            .json()
            .args(&["--bench", "--exact", "work/iter", "--discard-baseline"])
            .output();
        assert_success(&output);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("time:"), "no Criterion estimate:\n{stdout}");
        assert!(
            !stdout.contains('{'),
            "no hotpath report expected:\n{stdout}"
        );
        assert!(counts(&output).routine_calls > ITERATIONS);
    }

    // HOTPATH_CRITERION_ITERATIONS=1000 cargo run -p test-criterion --example fixture --features hotpath -- work/iter
    #[test]
    fn test_fixed_iterations_refuse_a_second_benchmark() {
        // A regex filter matching several benchmarks.
        let output = fixture("hotpath")
            .env("HOTPATH_CRITERION_ITERATIONS", "10")
            .args(&["work/iter"])
            .output();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("a second one started"), "{stderr}");

        // Criterion's benchmark mode enters the routine once per sample.
        let output = fixture("hotpath")
            .env("HOTPATH_CRITERION_ITERATIONS", "10")
            .args(&["--bench", "--exact", "work/iter", "--discard-baseline"])
            .output();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("a second one started"), "{stderr}");
    }

    // HOTPATH_CRITERION_ITERATIONS=1000 cargo run -p test-criterion --example fixture --features hotpath -- --exact missing
    #[test]
    fn test_fixed_iterations_fail_when_nothing_ran() {
        let output = fixed("hotpath", "missing").output();
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("no benchmark ran"), "{stderr}");
    }

    // HOTPATH_CRITERION_ITERATIONS=many cargo run -p test-criterion --example fixture --features hotpath -- --exact work/iter
    #[test]
    fn test_malformed_iterations_fail() {
        for value in ["many", "0", "-5"] {
            let output = fixture("hotpath")
                .env("HOTPATH_CRITERION_ITERATIONS", value)
                .args(&["--exact", "work/iter"])
                .output();
            assert!(!output.status.success(), "{value}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("HOTPATH_CRITERION_ITERATIONS: expected a positive integer"),
                "{value}: {stderr}"
            );
        }
    }

    // HOTPATH_BENCHMARK=work-iter HOTPATH_CRITERION_ITERATIONS=1000 HOTPATH_OUTPUT_FORMAT=json cargo run -p test-criterion --example fixture --features hotpath,hotpath-cloud -- --exact work/iter
    #[test]
    fn test_cloud_report_names_the_benchmark() {
        let output = fixed("hotpath,hotpath-cloud", "work/iter")
            .json()
            .env("HOTPATH_BENCHMARK", "work-iter")
            .env_remove("HOTPATH_UPLOAD")
            .env_remove("GITHUB_ACTIONS")
            .output();
        assert_success(&output);
        let report = parse_report(&String::from_utf8_lossy(&output.stdout));
        assert_eq!(report.meta.benchmark.as_deref(), Some("work-iter"));
        assert_eq!(report.meta.run, Some(hotpath::json::JsonRun::Native));
        let timing = report.functions_timing.expect("timing section");
        assert_eq!(entry(&timing, ROUTINE).calls, ITERATIONS);
    }

    fn trace(benchmark: &str) -> String {
        let output = Example::new("test-criterion", "batching")
            .env("HOTPATH_CRITERION_ITERATIONS", "25")
            .args(&["--exact", benchmark])
            .output();
        assert_success(&output);
        let stderr = String::from_utf8_lossy(&output.stderr);
        stderr
            .lines()
            .find_map(|l| l.strip_prefix("trace: "))
            .unwrap_or_else(|| panic!("no trace in stderr:\n{stderr}"))
            .to_string()
    }

    fn batches(size: usize, pattern: impl Fn(usize) -> String) -> String {
        (0..25)
            .step_by(size)
            .map(|done| pattern(size.min(25 - done)))
            .collect()
    }

    // HOTPATH_CRITERION_ITERATIONS=25 cargo run -p test-criterion --example batching --features hotpath -- --exact batched/value/10
    #[test]
    fn test_batches_follow_criterion() {
        let r = |c: &str, k: usize| c.repeat(k);
        assert_eq!(
            trace("batched/value/10"),
            batches(10, |k| r("S", k) + &r("RI", k) + &r("O", k))
        );
        assert_eq!(
            trace("batched/ref/10"),
            batches(10, |k| r("S", k) + &r("R", k) + &r("O", k) + &r("I", k))
        );
        assert_eq!(trace("batched/value/1"), r("SRIO", 25));
        assert_eq!(trace("batched/ref/1"), r("SROI", 25));
    }

    const MERGE_FEATURES: &str = "hotpath,hotpath-alloc,hotpath-callgrind,hotpath-cloud";

    fn merge_run(benchmark: &str, out: &str, callgrind: Option<&RunDir>) -> Example {
        let example = fixed(MERGE_FEATURES, benchmark)
            .release()
            .json()
            .env("HOTPATH_OUTPUT_PATH", out)
            .env("HOTPATH_BENCHMARK", "criterion-fixture")
            .env_remove("HOTPATH_UPLOAD")
            .env_remove("GITHUB_ACTIONS")
            .env_remove("GITHUB_STEP_SUMMARY");
        let Some(dir) = callgrind else {
            return example;
        };
        let out_file = dir.path("cg.%p");
        example
            .env(
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER",
                format!("valgrind -q --tool=callgrind --callgrind-out-file={out_file}"),
            )
            .env("HOTPATH_CALLGRIND_OUT_FILE", out_file)
    }

    fn read(path: &str) -> JsonReport {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn icount(benchmark: &str, dir: &RunDir) -> JsonFunctionsList {
        let out = dir.path(&format!(
            "icount-{}.json",
            dir.0.read_dir().unwrap().count()
        ));
        let output = merge_run(benchmark, &out, Some(dir)).output();
        assert_success(&output);
        assert_eq!(counts(&output).routine_calls, ITERATIONS);
        read(&out).functions_icount.expect("instruction counts")
    }

    // CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" HOTPATH_CRITERION_ITERATIONS=1000 HOTPATH_OUTPUT_FORMAT=json cargo run --release -p test-criterion --example fixture --features hotpath,hotpath-callgrind -- --exact work/iter_batched
    #[test]
    fn test_callgrind_counts_are_stable() {
        if !valgrind_available() {
            return;
        }
        let dir = RunDir::new();
        let first = icount("work/iter_batched", &dir);
        let second = icount("work/iter_batched", &dir);
        let total = |list: &JsonFunctionsList, name: &str| -> u64 {
            let e = entry(list, name);
            assert_eq!(e.calls, ITERATIONS, "{name}");
            e.total.parse().unwrap()
        };
        // A leaf measured function is exact.
        assert_eq!(total(&first, WORK), total(&second, WORK));
        // The routine also counts hotpath's own queueing, so it drifts slightly.
        let (a, b) = (total(&first, ROUTINE), total(&second, ROUTINE));
        let drift = a.abs_diff(b) as f64 / a as f64;
        assert!(drift < 1e-3, "routine instructions {a} vs {b}");
    }

    // HOTPATH_CRITERION_ITERATIONS=1000 HOTPATH_OUTPUT_FORMAT=json HOTPATH_OUTPUT_PATH=native.json cargo run --release -p test-criterion --example fixture --features hotpath,hotpath-alloc,hotpath-callgrind,hotpath-cloud -- --exact work/iter_batched
    // CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" HOTPATH_MERGE_REPORT=native.json HOTPATH_OUTPUT_PATH=merged.json ... (same command)
    #[test]
    fn test_callgrind_run_merges_the_native_report() {
        if !valgrind_available() {
            return;
        }
        let dir = RunDir::new();
        let (native_path, merged_path) = (dir.path("native.json"), dir.path("merged.json"));
        let native_output = merge_run("work/iter_batched", &native_path, None).output();
        assert_success(&native_output);
        let output = merge_run("work/iter_batched", &merged_path, Some(&dir))
            .env("HOTPATH_MERGE_REPORT", &native_path)
            .output();
        assert_success(&output);

        let (native, merged) = (read(&native_path), read(&merged_path));
        assert_eq!(merged.meta.run, Some(hotpath::json::JsonRun::Merged));
        assert_eq!(merged.meta.benchmark.as_deref(), Some("criterion-fixture"));
        let timing = merged.functions_timing.as_ref().expect("native timing");
        assert_eq!(
            timing.total_elapsed_ns,
            native.functions_timing.as_ref().unwrap().total_elapsed_ns
        );
        let alloc = merged.functions_alloc.as_ref().expect("native allocations");
        assert_eq!(bytes(&entry(alloc, ROUTINE).avg), 64 + 16);
        let icount = merged
            .functions_icount
            .as_ref()
            .expect("instruction counts");
        for list in [timing, alloc, icount] {
            assert_eq!(entry(list, ROUTINE).calls, ITERATIONS);
            assert_eq!(entry(list, WORK).calls, ITERATIONS);
        }
    }
}
