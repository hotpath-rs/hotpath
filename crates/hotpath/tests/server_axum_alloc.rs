//! Integration tests for per-route memory profiling: bytes allocated while a
//! handler future is polled are attributed to the matched route and reported
//! as a second `server` sub-table / the JSON `alloc` object.
//!
//! These run the `test-axum` `route_alloc` example as a subprocess with
//! `hotpath-alloc` and assert on the JSON report printed when the guard drops.
#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use hotpath::json::{JsonReport, JsonServerEntry};

    use crate::common::assert_contains_all;
    use crate::common::example::Example;

    const BIG_BYTES: f64 = 1024.0 * 1024.0;
    /// `GET /block` allocates 128 KiB before its `.await` and 256 KiB after.
    const BLOCK_BYTES: f64 = (128.0 + 256.0) * 1024.0;

    fn example(envs: &[(&str, &str)]) -> Example {
        Example::new("test-axum", "route_alloc")
            .features("hotpath-alloc")
            .envs(envs)
    }

    fn run_example(envs: &[(&str, &str)]) -> JsonReport {
        example(envs).json().report()
    }

    fn by_route<'a>(entries: &'a [JsonServerEntry], route: &str) -> &'a JsonServerEntry {
        entries
            .iter()
            .find(|e| e.route == route)
            .unwrap_or_else(|| panic!("{route} server entry missing: {entries:?}"))
    }

    #[test]
    fn test_route_alloc_attributes_bytes_per_route() {
        let report = run_example(&[]);
        let server = report.server.expect("No server section in report");
        assert!(
            server.total_alloc_bytes as f64 >= 3.0 * BIG_BYTES,
            "{server:?}"
        );

        let big = by_route(&server.data, "GET /big");
        assert_eq!(big.count, 3);
        let alloc = big.alloc.as_ref().expect("GET /big alloc missing");
        // The handler's 1 MiB body, plus whatever the framework adds.
        let bytes = alloc.bytes_per_request.expect("bytes_per_request");
        assert!(bytes >= BIG_BYTES, "{alloc:?}");
        assert!(alloc.allocs_per_request.unwrap() >= 1.0, "{alloc:?}");
        assert!(alloc.total_bytes as f64 >= 3.0 * BIG_BYTES, "{alloc:?}");
        assert!(alloc.percentiles.contains_key("p95"), "{alloc:?}");

        // Framework overhead only: far below the 1 MiB body.
        let small = by_route(&server.data, "GET /small");
        assert_eq!(small.count, 4);
        let alloc = small.alloc.as_ref().expect("GET /small alloc missing");
        let bytes = alloc.bytes_per_request.expect("bytes_per_request");
        assert!(bytes < 64.0 * 1024.0, "{alloc:?}");

        // Unmatched requests carry no route scope, so no memory.
        let missing = by_route(&server.data, "GET <unmatched>");
        assert_eq!(missing.count, 1);
        let alloc = missing.alloc.as_ref().expect("unmatched alloc object");
        assert_eq!(alloc.bytes_per_request, None, "{alloc:?}");
        assert_eq!(alloc.allocs_per_request, None, "{alloc:?}");
        assert_eq!(alloc.total_bytes, 0, "{alloc:?}");
    }

    /// A `measure_block!` spanning an `.await` leaves the block's allocation
    /// frame open when the route scope closes for that poll. Both halves count
    /// towards the route, and neither counts twice.
    #[test]
    fn test_route_alloc_counts_block_spanning_await() {
        let report = run_example(&[]);
        let server = report.server.expect("No server section in report");

        let block = by_route(&server.data, "GET /block");
        assert_eq!(block.count, 2);
        let alloc = block.alloc.as_ref().expect("GET /block alloc missing");
        let bytes = alloc.bytes_per_request.expect("bytes_per_request");
        assert!(bytes >= BLOCK_BYTES, "{alloc:?}");
        assert!(bytes < 1.5 * BLOCK_BYTES, "{alloc:?}");
    }

    /// The route counts the block's bytes inclusively while the block itself
    /// keeps reporting them as its own exclusive total: the scope adds no frame
    /// to the allocation stack, so function accounting is unchanged.
    #[test]
    fn test_route_alloc_block_keeps_function_totals_exclusive() {
        let report = run_example(&[("HOTPATH_REPORT", "server,functions-alloc")]);
        let functions = report
            .functions_alloc
            .expect("No functions_alloc section in report");
        let block = functions
            .data
            .iter()
            .find(|f| f.name == "block_await")
            .unwrap_or_else(|| panic!("block_await missing: {:?}", functions.data));
        assert_eq!(block.calls, 2, "{block:?}");
        // 128 KiB before the await plus 256 KiB after, both in the block's own
        // frame, counted once.
        assert!(
            block.avg.starts_with("384.") && block.avg.ends_with("KB"),
            "{block:?}"
        );

        let server = report.server.expect("No server section in report");
        let alloc = by_route(&server.data, "GET /block")
            .alloc
            .as_ref()
            .expect("GET /block alloc missing")
            .clone();
        assert!(alloc.bytes_per_request.unwrap() >= BLOCK_BYTES, "{alloc:?}");
    }

    /// `HOTPATH_ALLOC_CUMULATIVE=1` makes function frames inclusive by
    /// propagating each pop into its parent. Route totals are inclusive either
    /// way, so they must not pick the same bytes up twice.
    #[test]
    fn test_route_alloc_cumulative_mode_does_not_double_count() {
        let report = run_example(&[("HOTPATH_ALLOC_CUMULATIVE", "1")]);
        let server = report.server.expect("No server section in report");

        let big = by_route(&server.data, "GET /big")
            .alloc
            .as_ref()
            .expect("GET /big alloc missing")
            .clone();
        let bytes = big.bytes_per_request.expect("bytes_per_request");
        assert!(bytes >= BIG_BYTES && bytes < 1.5 * BIG_BYTES, "{big:?}");

        let block = by_route(&server.data, "GET /block")
            .alloc
            .as_ref()
            .expect("GET /block alloc missing")
            .clone();
        let bytes = block.bytes_per_request.expect("bytes_per_request");
        assert!(
            bytes >= BLOCK_BYTES && bytes < 1.5 * BLOCK_BYTES,
            "{block:?}"
        );
    }

    #[test]
    fn test_route_alloc_server_table_columns() {
        let stdout = example(&[]).stdout();
        assert_contains_all(&stdout, &["Allocs/req", "GET /big", "GET /small"]);
        // Second occurrence of the route is the memory sub-table row:
        // Route | Calls | Allocs/req | Avg | ... | Total | % Total
        let big_rows: Vec<&str> = stdout.lines().filter(|l| l.contains("GET /big")).collect();
        assert_eq!(big_rows.len(), 2, "{stdout}");
        let cells: Vec<&str> = big_rows[1].split('|').map(str::trim).collect();
        assert_eq!(&cells[1..3], &["GET /big", "3"], "{}", big_rows[1]);
        assert!(cells[4].ends_with(" MB"), "avg bytes: {}", big_rows[1]);

        let missing_rows: Vec<&str> = stdout
            .lines()
            .filter(|l| l.contains("GET <unmatched>"))
            .collect();
        assert_eq!(missing_rows.len(), 2, "{stdout}");
        let cells: Vec<&str> = missing_rows[1].split('|').map(str::trim).collect();
        assert_eq!(
            &cells[1..5],
            &["GET <unmatched>", "1", "-", "-"],
            "{}",
            missing_rows[1]
        );
    }

    #[test]
    fn test_route_alloc_nested_layer_reports_once() {
        let report = run_example(&[("NESTED", "1")]);
        let server = report.server.expect("No server section in report");
        // Three requests through two layers: counted once, by the outer one,
        // with the nested handler's bytes.
        let nested = by_route(&server.data, "GET /nested/big");
        assert_eq!(nested.count, 3, "{:?}", server.data);
        let total: u64 = server.data.iter().map(|e| e.count).sum();
        assert_eq!(total, 4, "{:?}", server.data);
        let alloc = nested.alloc.as_ref().expect("nested alloc missing");
        assert!(alloc.bytes_per_request.unwrap() >= BIG_BYTES, "{alloc:?}");

        // Still counted once when no route scope exists at all.
        let report = run_example(&[("NESTED", "1"), ("HOTPATH_ROUTE_SCOPE", "0")]);
        let server = report.server.expect("No server section in report");
        assert_eq!(
            by_route(&server.data, "GET /nested/big").count,
            3,
            "{:?}",
            server.data
        );
        let total: u64 = server.data.iter().map(|e| e.count).sum();
        assert_eq!(total, 4, "{:?}", server.data);
    }

    #[test]
    fn test_route_alloc_disabled_scope_reports_nothing() {
        let report = run_example(&[("HOTPATH_ROUTE_SCOPE", "0")]);
        let server = report.server.expect("No server section in report");
        assert_eq!(server.total_alloc_bytes, 0, "{server:?}");
        for entry in &server.data {
            let alloc = entry.alloc.as_ref().expect("alloc object present");
            assert_eq!(alloc.bytes_per_request, None, "{entry:?}");
            assert_eq!(alloc.total_bytes, 0, "{entry:?}");
        }
    }
}
