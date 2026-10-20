#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    use hotpath::json::{JsonFunctionsList, JsonReport};

    use crate::common::example::{assert_success, Example};
    use crate::common::report::parse_report;

    const FEATURES: &str = "hotpath,hotpath-callgrind";

    /// Valgrind on x86_64 Linux. Locally the Valgrind tests skip without it;
    /// on CI (`CI` set) a missing Valgrind fails them.
    fn valgrind_available() -> bool {
        let supported = cfg!(all(target_arch = "x86_64", target_os = "linux"))
            && Command::new("valgrind")
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success());
        if !supported {
            assert!(
                std::env::var_os("CI").is_none(),
                "valgrind is required on CI"
            );
            eprintln!("skipping: needs valgrind on x86_64 Linux");
        }
        supported
    }

    /// A fresh directory for one run's dumps and reports, removed on drop.
    struct RunDir(PathBuf);

    impl RunDir {
        fn new() -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("hotpath-callgrind-{nanos}"));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self, name: &str) -> String {
            self.0.join(name).to_str().unwrap().to_string()
        }
    }

    impl Drop for RunDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The example run under Callgrind, its dumps written to `dir` through a
    /// custom `--callgrind-out-file` that `HOTPATH_CALLGRIND_OUT_FILE` names.
    fn under_callgrind(example: &str, dir: &RunDir) -> Example {
        let out_file = dir.path("cg.%p");
        Example::new("test-callgrind", example)
            .features(FEATURES)
            .release()
            .env("HOTPATH_REPORT", "auto,-threads")
            .env(
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER",
                format!("valgrind -q --tool=callgrind --callgrind-out-file={out_file}"),
            )
            .env("HOTPATH_CALLGRIND_OUT_FILE", out_file)
    }

    /// The report and stderr of one Callgrind run of `instructions`.
    fn callgrind_report() -> (JsonReport, String) {
        let dir = RunDir::new();
        let output = under_callgrind("instructions", &dir).json().output();
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

    // CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="valgrind -q --tool=callgrind" HOTPATH_OUTPUT_FORMAT=json cargo run --release -p test-callgrind --example instructions --features hotpath,hotpath-callgrind
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
        let list = first.functions_instructions.expect("instructions section");
        assert!(
            list.tool
                .as_deref()
                .is_some_and(|t| t.starts_with("callgrind-")),
            "{:?}",
            list.tool
        );

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
        for (name, calls, spins) in expected {
            let (actual_calls, actual) = total(&list, name);
            assert_eq!(actual_calls, calls, "{name} calls");
            let ratio = actual as f64 / (unit * spins) as f64;
            assert!(
                (0.999..1.001).contains(&ratio),
                "{name}: {actual} Ir, expected about {spins} x {unit}"
            );
        }
        assert_eq!(list.data.len(), expected.len(), "{:#?}", list.data);
        // alpha::generated and beta::generated share a marker: excluded and
        // reported, never merged into one count.
        assert!(
            stderr.contains("no instruction counts for")
                && stderr.contains("instructions::alpha::generated")
                && stderr.contains("instructions::beta::generated"),
            "{stderr}"
        );

        let second = second.functions_instructions.expect("instructions section");
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
            report.functions_instructions.expect("instructions section")
        };
        let (first_calls, first_total) = total(&read(&first), "sessions::work");
        let (second_calls, second_total) = total(&read(&second), "sessions::work");

        assert_eq!((first_calls, second_calls), (1, 3));
        let ratio = second_total as f64 / (3 * first_total) as f64;
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

        assert!(report.functions_instructions.is_none());
        let timing = report.functions_timing.expect("timing section");
        assert!(timing
            .data
            .iter()
            .any(|e| e.name == "instructions::free_fn"));
    }
}
