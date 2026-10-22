#[path = "common/support.rs"]
mod common;

#[cfg(test)]
pub mod tests {
    use crate::common::assert_contains_all;
    use crate::common::example::Example;

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
        let stdout = example("cpu_symbols").json().stdout();
        let json_line = stdout.lines().last().expect("no output");

        let expected_symbols = [
            "cpu_symbols::free_heavy_work",
            "Worker::method_heavy_work",
            "Worker::method_light_work",
            "OtherWorker::method_heavy_work",
        ];

        for expected in expected_symbols {
            assert!(
                json_line.contains(expected),
                "Expected symbol:\n{expected}\n\nGot:\n{json_line}",
            );
        }
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
