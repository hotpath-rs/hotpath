#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
mod tests {
    use hotpath::json::{JsonLocation, JsonReport};

    use crate::common::example::Example;

    const EXAMPLE_FILE: &str = "crates/test-tokio-async/examples/locations.rs";

    fn run_example(source_root: Option<&str>) -> JsonReport {
        let mut example = Example::new("test-tokio-async", "locations");
        if let Some(root) = source_root {
            example = example.env("HOTPATH_SOURCE_ROOT", root);
        }
        example.report()
    }

    /// 1-indexed line of the first line containing `needle` in the example
    /// source, so assertions track the file without hardcoding line numbers.
    fn example_line_of(needle: &str) -> u32 {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../test-tokio-async/examples/locations.rs"
        );
        let source = std::fs::read_to_string(path).expect("example source readable");
        source
            .lines()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not found in example")) as u32
            + 1
    }

    fn assert_example_location(location: &JsonLocation, context: &str) {
        assert_eq!(location.file, EXAMPLE_FILE, "{context}");
        assert!(location.line > 0, "{context}: line must be set");
        assert!(location.column > 0, "{context}: column must be set");
    }

    #[test]
    fn test_locations_report() {
        let report = run_example(None);

        // Functions: plain, labeled, measure_all method, measure_block!, and
        // the #[hotpath::main] wrapper all carry a location.
        let functions = report.functions_timing.expect("functions_timing section");
        let entry = |name: &str| {
            functions
                .data
                .iter()
                .find(|e| e.name == name)
                .unwrap_or_else(|| panic!("no entry named {name}"))
        };

        let plain = entry("locations::plain_function");
        let location = plain.location.as_ref().expect("plain_function location");
        assert_example_location(location, "plain_function");
        assert_eq!(location.line, example_line_of("fn plain_function"));

        let labeled = entry("custom_label_fn");
        let location = labeled.location.as_ref().expect("labeled location");
        assert_example_location(location, "labeled_function");
        assert_eq!(location.line, example_line_of("fn labeled_function"));

        let method = entry("locations::Worker::run");
        let location = method.location.as_ref().expect("Worker::run location");
        assert_example_location(location, "Worker::run");
        assert_eq!(location.line, example_line_of("fn run"));

        let block = entry("locations_block");
        let location = block.location.as_ref().expect("measure_block location");
        assert_example_location(location, "measure_block");
        assert_eq!(location.line, example_line_of("measure_block!"));

        // One measure_block! call site producing two distinct labels: both
        // entries must resolve to that call site, not just the first label
        // registered.
        for name in ["block_a", "block_b"] {
            let block = entry(name);
            let location = block.location.as_ref().expect(name);
            assert_example_location(location, name);
            assert_eq!(location.line, example_line_of("block_a"), "{name}");
        }

        // A `#[path = "../support/..."]` module: `file!()` keeps the `..`
        // segment, the report resolves it to a repository path.
        let path_module = entry("locations::path_module::path_module_function");
        let location = path_module.location.as_ref().expect("path module location");
        assert_eq!(
            location.file,
            "crates/test-tokio-async/support/path_module.rs"
        );
        let fn_line = include_str!("../../test-tokio-async/support/path_module.rs")
            .lines()
            .position(|l| l.contains("fn path_module_function"))
            .expect("fn in path module") as u32
            + 1;
        assert_eq!(location.line, fn_line);

        let main_entry = entry("locations::main");
        let location = main_entry.location.as_ref().expect("main location");
        assert_example_location(location, "main");

        // Resource entries: location matches the call line and the `source`
        // string stays the `file:line` form (identity regression guard).
        let mutexes = report.mutexes.expect("mutexes section");
        let mutex = &mutexes.data[0];
        let location = &mutex.location;
        assert_example_location(location, "mutex");
        assert_eq!(location.line, example_line_of("hotpath::mutex!"));
        assert_eq!(
            mutex.source,
            format!("{}:{}", location.file, location.line),
            "mutex source must stay file:line"
        );

        let channels = report.channels.expect("channels section");
        let channel = &channels.data[0];
        let location = &channel.location;
        assert_example_location(location, "channel");
        assert_eq!(location.line, example_line_of("hotpath::channel!"));
        assert_eq!(
            channel.source,
            format!("{}:{}", location.file, location.line),
            "channel source must stay file:line"
        );

        // future! ids are `file:line:column` like every other resource macro;
        // the displayed source strips the column back to `file:line`.
        let futures = report.futures.expect("futures section");
        let future = &futures.data[0];
        let location = &future.location;
        assert_example_location(location, "future");
        assert_eq!(location.line, example_line_of("hotpath::future!"));
        assert_eq!(
            future.source,
            format!("{}:{}", location.file, location.line),
            "future source must stay file:line"
        );

        // source_root resolves against the build workspace root (the repo root here),
        // not the child's cwd (the crate dir).
        let meta = report.meta;
        assert!(!meta.rustc.is_empty(), "rustc version");
        assert!(meta.os.contains('-'), "os is <os>-<arch>: {}", meta.os);
        assert!(meta.created_at.offset().is_utc());
        let age = time::OffsetDateTime::now_utc() - meta.created_at;
        assert!(
            age >= time::Duration::ZERO && age < time::Duration::hours(1),
            "created_at is recent: {} (age {age})",
            meta.created_at
        );
        assert_eq!(meta.source_root.as_deref(), Some(""));
    }

    #[test]
    fn test_source_root_env_override() {
        let report = run_example(Some("crates/test-tokio-async"));
        assert_eq!(
            report.meta.source_root.as_deref(),
            Some("crates/test-tokio-async")
        );
    }
}
