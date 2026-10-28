#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use std::path::Path;
    use std::process::Output;

    use hotpath::json::{JsonFunctionsList, JsonReport};

    use crate::common::callgrind::{valgrind_available, RunDir};
    use crate::common::example::{assert_success, Example};
    use crate::common::report::parse_report;

    const FEATURES: &str = "hotpath,hotpath-callgrind";

    /// The example run under Callgrind, its dumps written to `dir` through a
    /// custom `--callgrind-out-file` that `HOTPATH_CALLGRIND_OUT_FILE` names.
    fn under_callgrind(example: &str, dir: &RunDir) -> Example {
        let out_file = dir.path("cg.%p");
        Example::new("test-callgrind", example)
            .features(FEATURES)
            .release()
            .env("HOTPATH_REPORT", "auto,-threads")
            .env("HOTPATH_FUNCTIONS_LIMIT", "0")
            .env(
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER",
                format!("valgrind -q --tool=callgrind --callgrind-out-file={out_file}"),
            )
            .env("HOTPATH_CALLGRIND_OUT_FILE", out_file)
    }

    /// The report and stderr of one Callgrind run of `instructions`. The
    /// sampling rate is overridden to count-only under Valgrind; a fractional
    /// one left in place makes `recursive`'s count vary between runs.
    fn callgrind_report() -> (JsonReport, String) {
        let dir = RunDir::new();
        let output = under_callgrind("instructions", &dir)
            .env("HOTPATH_TIME_SAMPLING_RATE", "0.5")
            .json()
            .output();
        assert_success(&output);
        (
            parse_report(&String::from_utf8_lossy(&output.stdout)),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    fn total(list: &JsonFunctionsList, name: &str) -> (u64, u64) {
        let entry = list
            .data
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name} missing from {:#?}", list.data));
        (entry.calls, entry.total.parse().unwrap())
    }

    // CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" HOTPATH_TIME_SAMPLING_RATE=0.5 HOTPATH_OUTPUT_FORMAT=json cargo run --release -p test-callgrind --example instructions --features hotpath,hotpath-callgrind
    #[test]
    fn test_instructions_are_exact_and_attributed() {
        if !valgrind_available() {
            return;
        }
        let (first, stderr) = callgrind_report();
        let (second, _) = callgrind_report();

        assert!(
            first.functions_timing.is_none(),
            "timing is meaningless under Valgrind"
        );
        let rates = first.time_sampling.as_ref().expect("time sampling rates");
        assert!(
            !rates.is_empty() && rates.values().all(|&r| r == 0.0),
            "{rates:?}"
        );
        let list = first.functions_icount.expect("instructions section");
        assert!(
            list.tool
                .as_deref()
                .is_some_and(|t| t.starts_with("callgrind-")),
            "{:?}",
            list.tool
        );
        // The run total covers every measured body, so it bounds each
        // inclusive count.
        let total_icount = list.total_icount.expect("total_icount");
        for entry in &list.data {
            let total: u64 = entry.total.parse().unwrap();
            assert!(
                total <= total_icount,
                "{}: {total} > {total_icount}",
                entry.name
            );
        }

        // Every measured body runs one `spin(N)` per call, except recursive
        // (4 levels, each level's inclusive cost counted: 4+3+2+1) and
        // async_fn (4 spins per call).
        let (_, unit) = total(&list, "instructions::logged_fn");
        let expected = [
            ("instructions::free_fn", 2, 2),
            ("instructions::generic_fn", 2, 2),
            ("instructions::recursive", 4, 10),
            ("labeled", 1, 1),
            ("instructions::logged_fn", 1, 1),
            ("instructions::Worker::inherent", 1, 1),
            ("instructions::Worker::first_mut", 1, 1),
            ("instructions::run", 1, 1),
            ("instructions::async_fn", 2, 8),
            ("instructions::tracked_future", 1, 1),
            ("instructions::nested::first", 1, 1),
            ("instructions::nested::second", 1, 1),
        ];
        // `avg` is `total / calls` to the hundredth, rounded half-up, not a
        // truncated whole number.
        for entry in &list.data {
            let total: u128 = entry.total.parse().unwrap();
            let calls = u128::from(entry.calls);
            let hundredths = u64::try_from((total * 100 + calls / 2) / calls).unwrap();
            assert_eq!(
                entry.avg,
                hotpath::format_icount(hundredths),
                "{} avg",
                entry.name
            );
            assert_eq!(hotpath::parse_icount(&entry.avg), Some(hundredths));
        }

        for (name, calls, spins) in expected {
            let (actual_calls, actual) = total(&list, name);
            assert_eq!(actual_calls, calls, "{name} calls");
            let ratio = actual as f64 / (unit * spins) as f64;
            assert!(
                (0.999..1.001).contains(&ratio),
                "{name}: {actual} Ir, expected about {spins} x {unit}"
            );
        }
        // Return type shapes the marker closure must compile for, each
        // attributed once; they run no `spin`, so no ratio to check.
        let shapes = [
            "instructions::Worker::values",
            "instructions::logged_slice",
            "instructions::Worker::evens",
            "instructions::Worker::cloned",
            "instructions::Worker::parse_first",
        ];
        for name in shapes {
            let (calls, total) = total(&list, name);
            assert_eq!(calls, 1, "{name} calls");
            assert!(total > 0, "{name} has no instructions");
        }
        assert_eq!(
            list.data.len(),
            expected.len() + shapes.len(),
            "{:#?}",
            list.data
        );
        // alpha::generated and beta::generated share a marker: excluded and
        // reported, never merged into one count.
        assert!(
            stderr.contains("no instruction counts for")
                && stderr.contains("instructions::alpha::generated")
                && stderr.contains("instructions::beta::generated"),
            "{stderr}"
        );

        let second = second.functions_icount.expect("instructions section");
        for (name, _, _) in expected {
            assert_eq!(
                total(&list, name),
                total(&second, name),
                "{name} differs between runs"
            );
        }
    }

    // CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" cargo run --release -p test-callgrind --example sessions --features hotpath,hotpath-callgrind -- first.json second.json
    #[test]
    fn test_each_session_counts_only_its_own_calls() {
        if !valgrind_available() {
            return;
        }
        let dir = RunDir::new();
        let (first, second) = (dir.path("first.json"), dir.path("second.json"));
        under_callgrind("sessions", &dir)
            .args(&[&first, &second])
            .stdout();

        let read = |path: &str| -> JsonFunctionsList {
            let report: JsonReport =
                serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            report.functions_icount.expect("instructions section")
        };
        let (first_calls, first_total) = total(&read(&first), "sessions::work");
        let (second_calls, second_total) = total(&read(&second), "sessions::work");

        // 3 calls in the session plus one from its `before_shutdown`.
        assert_eq!((first_calls, second_calls), (1, 4));
        let ratio = second_total as f64 / (4 * first_total) as f64;
        assert!(
            (0.999..1.001).contains(&ratio),
            "{first_total} then {second_total} Ir"
        );
    }

    // HOTPATH_OUTPUT_FORMAT=json cargo run --release -p test-callgrind --example instructions --features hotpath,hotpath-callgrind
    #[test]
    fn test_native_run_reports_timing() {
        let report = Example::new("test-callgrind", "instructions")
            .features(FEATURES)
            .release()
            .json()
            .report();

        assert!(report.functions_icount.is_none());
        let timing = report.functions_timing.expect("timing section");
        assert!(timing
            .data
            .iter()
            .any(|e| e.name == "instructions::free_fn"));
    }

    const MERGE_FEATURES: &str = "hotpath,hotpath-callgrind,hotpath-cloud";
    /// What a merge output file holds before a run; a refused merge must
    /// leave it byte-for-byte.
    const PREVIOUS: &str = "previous report\n";

    /// `example` built for merging, writing JSON to `out`. Actions variables
    /// are cleared so the child neither annotates this job nor differs from
    /// the native run it is compared with.
    fn merge_example(example: Example, out: &str) -> Example {
        example
            .json()
            .env("HOTPATH_OUTPUT_PATH", out)
            .env_remove("GITHUB_ACTIONS")
            .env_remove("GITHUB_STEP_SUMMARY")
            .env_remove("HOTPATH_UPLOAD")
    }

    fn native_example(example: &str) -> Example {
        Example::new("test-callgrind", example)
            .features(MERGE_FEATURES)
            .release()
    }

    /// A native run of `instructions` writing its JSON report to `path`.
    fn write_native_report(path: &str) {
        let output = merge_example(native_example("instructions"), path).output();
        assert_success(&output);
    }

    /// The merge was refused: exit 1, the reason on stderr, the output file
    /// untouched and no temporary file left beside it.
    #[track_caller]
    fn assert_merge_refused(output: &Output, out: &str, reason: &str) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
        assert!(
            stderr.contains("merge failed: ") && stderr.contains(reason),
            "expected {reason:?} in stderr:\n{stderr}"
        );
        assert_eq!(std::fs::read_to_string(out).unwrap(), PREVIOUS);
        assert_no_temp_files(Path::new(out).parent().unwrap());
    }

    #[track_caller]
    fn assert_no_temp_files(dir: &Path) {
        let temps: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(temps.is_empty(), "temporary files left: {temps:?}");
    }

    /// A native run of the merging build, refused for each file-side reason
    /// and for not running under Callgrind; all of it runs without Valgrind.
    // HOTPATH_OUTPUT_FORMAT=json HOTPATH_OUTPUT_PATH=out.json HOTPATH_MERGE_REPORT=native.json cargo run --release -p test-callgrind --example instructions --features hotpath,hotpath-callgrind,hotpath-cloud
    #[test]
    fn test_refused_merge_fails_and_leaves_output_untouched() {
        let dir = RunDir::new();
        let (native, out) = (dir.path("native.json"), dir.path("out.json"));
        write_native_report(&native);
        let refused = |merge_report: &str, extra: &[(&str, &str)]| -> Output {
            std::fs::write(&out, PREVIOUS).unwrap();
            merge_example(native_example("instructions"), &out)
                .env("HOTPATH_MERGE_REPORT", merge_report)
                .envs(extra)
                .output()
        };

        // A valid native report, but this run is not under Callgrind.
        assert_merge_refused(&refused(&native, &[]), &out, "not under valgrind");

        let missing = dir.path("missing.json");
        assert_merge_refused(
            &refused(&missing, &[]),
            &out,
            "cannot read the native report",
        );

        let malformed = dir.path("malformed.json");
        std::fs::write(&malformed, "{\"type\":").unwrap();
        assert_merge_refused(&refused(&malformed, &[]), &out, "not a hotpath JSON report");

        let huge = dir.path("huge.json");
        std::fs::write(
            &huge,
            vec![b' '; hotpath::json::cloud_api::MAX_REPORT_BYTES + 1],
        )
        .unwrap();
        assert_merge_refused(
            &refused(&huge, &[]),
            &out,
            "over the 5242880 byte report limit",
        );

        assert_merge_refused(
            &refused(&native, &[("HOTPATH_BENCHMARK", "other-bench")]),
            &out,
            "benchmark",
        );

        // The output file is the native report itself: refused before
        // anything is read, and the native report survives.
        let before = std::fs::read(&native).unwrap();
        let output = merge_example(native_example("instructions"), &native)
            .env("HOTPATH_MERGE_REPORT", &native)
            .output();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
        assert!(stderr.contains("both name"), "stderr:\n{stderr}");
        assert_eq!(std::fs::read(&native).unwrap(), before);

        // Table output and no upload: nothing to merge into.
        std::fs::write(&out, PREVIOUS).unwrap();
        let output = merge_example(native_example("instructions"), &out)
            .env("HOTPATH_OUTPUT_FORMAT", "table")
            .env("HOTPATH_MERGE_REPORT", &native)
            .output();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
        assert!(
            stderr.contains("nothing to merge into"),
            "stderr:\n{stderr}"
        );
    }

    /// A workload that panics skips the merge: no report from the partial
    /// run, the output untouched, the panic's own exit status.
    // HOTPATH_OUTPUT_FORMAT=json HOTPATH_OUTPUT_PATH=out.json HOTPATH_MERGE_REPORT=native.json cargo run --release -p test-callgrind --example panics --features hotpath,hotpath-callgrind,hotpath-cloud
    #[test]
    fn test_panicking_run_skips_the_merge() {
        let dir = RunDir::new();
        let out = dir.path("out.json");
        std::fs::write(&out, PREVIOUS).unwrap();
        let output = merge_example(native_example("panics"), &out)
            .env("HOTPATH_MERGE_REPORT", dir.path("native.json"))
            .output();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "stderr:\n{stderr}");
        assert_ne!(output.status.code(), Some(1), "stderr:\n{stderr}");
        assert!(stderr.contains("merge skipped"), "stderr:\n{stderr}");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), PREVIOUS);
        assert_no_temp_files(Path::new(&out).parent().unwrap());
    }

    /// The Callgrind run merges the native report: native timing and
    /// allocations, its own instruction counts, `meta.run` merged.
    // HOTPATH_OUTPUT_FORMAT=json HOTPATH_OUTPUT_PATH=native.json cargo run --release -p test-callgrind --example instructions --features hotpath,hotpath-callgrind,hotpath-cloud
    // CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" HOTPATH_OUTPUT_FORMAT=json HOTPATH_OUTPUT_PATH=out.json HOTPATH_MERGE_REPORT=native.json cargo run --release -p test-callgrind --example instructions --features hotpath,hotpath-callgrind,hotpath-cloud
    #[test]
    fn test_callgrind_run_merges_the_native_report() {
        if !valgrind_available() {
            return;
        }
        let dir = RunDir::new();
        let (native_path, out) = (dir.path("native.json"), dir.path("out.json"));
        write_native_report(&native_path);
        std::fs::write(&out, PREVIOUS).unwrap();

        let output = merge_example(
            under_callgrind("instructions", &dir).features(MERGE_FEATURES),
            &out,
        )
        .env("HOTPATH_MERGE_REPORT", &native_path)
        .output();
        assert_success(&output);
        assert_no_temp_files(&dir.0);

        let read = |path: &str| -> JsonReport {
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let (native, merged) = (read(&native_path), read(&out));
        assert_eq!(merged.meta.run, Some(hotpath::json::JsonRun::Merged));
        let timing = merged.functions_timing.as_ref().expect("native timing");
        assert_eq!(
            timing.total_elapsed_ns,
            native.functions_timing.as_ref().unwrap().total_elapsed_ns,
            "timing is the native run's"
        );
        let list = merged.functions_icount.expect("instruction counts");
        assert!(list
            .tool
            .as_deref()
            .is_some_and(|t| t.starts_with("callgrind-")));
        let (calls, _) = total(&list, "instructions::free_fn");
        assert_eq!(calls, 2);
    }

    /// Under Callgrind with an out-file hotpath cannot find: no counts, so
    /// the merge is refused with the dump's reason.
    #[test]
    fn test_unreadable_dump_refuses_the_merge() {
        if !valgrind_available() {
            return;
        }
        let dir = RunDir::new();
        let (native, out) = (dir.path("native.json"), dir.path("out.json"));
        write_native_report(&native);
        std::fs::write(&out, PREVIOUS).unwrap();

        let output = merge_example(
            under_callgrind("instructions", &dir).features(MERGE_FEATURES),
            &out,
        )
        .env("HOTPATH_CALLGRIND_OUT_FILE", dir.path("elsewhere.%p"))
        .env("HOTPATH_MERGE_REPORT", &native)
        .output();
        assert_merge_refused(&output, &out, "failed to read Callgrind dump");
    }
}
