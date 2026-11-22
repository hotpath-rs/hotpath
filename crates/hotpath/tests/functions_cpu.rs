#[path = "common/support.rs"]
mod common;

#[cfg(test)]
pub mod tests {
    use crate::common::example::Example;
    use crate::common::{assert_contains_all, first_json_value};

    fn example(name: &str) -> Example {
        Example::new("test-tokio-async", name)
            .features("hotpath-cpu")
            .env("HOTPATH_REPORT", "functions-cpu")
    }

    // cargo run -p test-tokio-async --example cpu_basic --features hotpath-cpu
    #[test]
    fn test_cpu_basic_output() {
        let stdout = example("cpu_basic").stdout();

        assert_contains_all(&stdout, &["cpu_basic::heavy_work", "cpu_basic::light_work"]);
    }

    // HOTPATH_OUTPUT_FORMAT=json HOTPATH_REPORT=functions-cpu cargo run -p test-tokio-async --example cpu_symbols --features hotpath-cpu
    #[test]
    fn test_cpu_symbols_output() {
        let output = example("cpu_symbols").json().output();
        crate::common::example::assert_success(&output);
        let report = first_json_value(&String::from_utf8_lossy(&output.stdout));
        let rows = report["functions_cpu"]["data"]
            .as_array()
            .unwrap_or_else(|| panic!("no CPU rows: {report}"));
        let samples = |name: &str| {
            rows.iter()
                .find(|r| r["name"] == name)
                .and_then(|r| r["samples"].as_u64())
                .unwrap_or(0)
        };

        let expected = [
            "free_heavy_work",
            "Worker::method_heavy_work",
            "Worker::method_light_work",
            "OtherWorker::method_heavy_work",
            "OtherWorker::receiver_heavy_work",
            "<OtherWorker as Heavy>::work",
            "<OtherWorker as Light>::work",
            "Generic::generic_work",
            "Defaulted::default_work",
            "<_ as Blanket>::blanket_work",
        ];
        for name in expected {
            let name = format!("cpu_symbols::{name}");
            assert!(samples(&name) > 0, "no CPU samples for {name}: {report}");
        }
        // Every measured function has a symbol; the unmeasured override of
        // `default_work` is left out, not credited to the default method.
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("no symbol of this binary maps"),
            "{stderr}"
        );
        assert_eq!(rows.len(), expected.len(), "{report}");
    }

    // cargo run -p test-tokio-async --example cpu_inline --features hotpath-cpu
    #[test]
    fn test_cpu_inline_default_strips_user_inline() {
        let stdout = example("cpu_inline")
            .release()
            .env("CARGO_TARGET_DIR", "target/test-cpu-inline-default")
            .stdout();

        assert_contains_all(
            &stdout,
            &["cpu_inline::never_inlined", "cpu_inline::always_inlined"],
        );
    }

    // HOTPATH_KEEP_INLINE=1 cargo run -p test-tokio-async --example cpu_inline --features hotpath-cpu
    #[test]
    fn test_cpu_inline_keep_inline_preserves_user_inline() {
        let stdout = example("cpu_inline")
            .release()
            .env("HOTPATH_KEEP_INLINE", "1")
            .env("CARGO_TARGET_DIR", "target/test-cpu-inline-keep")
            .stdout();

        assert!(
            stdout.contains("cpu_inline::never_inlined"),
            "Expected:\ncpu_inline::never_inlined\n\nGot:\n{stdout}",
        );
        assert!(
            !stdout.contains("cpu_inline::always_inlined"),
            "Expected always_inlined to be missing under HOTPATH_KEEP_INLINE=1.\n\nGot:\n{stdout}",
        );
    }

    // cargo run -p test-tokio-async --example cpu_labels --features hotpath-cpu
    #[test]
    fn test_cpu_labels_output() {
        let stdout = example("cpu_labels").stdout();

        assert_contains_all(&stdout, &["custom_heavy", "cpu_labels::heavy_no_label"]);
    }
}
