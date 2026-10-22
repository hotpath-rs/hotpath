#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
mod tests {
    use hotpath::json::JsonReport;
    use hotpath::{parse_bytes, parse_duration};

    use crate::common::example::Example;

    fn run_with_features(features: &str) -> JsonReport {
        Example::new("test-all-features", "basic_all_features")
            .features(features)
            .json()
            .env_remove("HOTPATH_UPLOAD")
            .env_remove("HOTPATH_ALLOC_METRIC")
            .env_remove("ACTIONS_ID_TOKEN_REQUEST_URL")
            .env_remove("ACTIONS_ID_TOKEN_REQUEST_TOKEN")
            .report()
    }

    /// `(unit, decimals)` of a formatted duration such as `1.004999 ms`.
    fn duration_shape(s: &str) -> (&str, usize) {
        let (number, unit) = s
            .rsplit_once(' ')
            .unwrap_or_else(|| panic!("no unit in {s:?}"));
        let decimals = number.rsplit_once('.').map_or(0, |(_, frac)| frac.len());
        (unit, decimals)
    }

    fn assert_exact_duration(s: &str, what: &str) {
        let expected = match duration_shape(s) {
            ("ns", d) => d == 0,
            ("µs", d) => d == 3,
            ("ms", d) => d == 6,
            ("s", d) => d == 9,
            _ => false,
        };
        assert!(expected, "{what}: {s:?} is not in the exact cloud format");
        assert!(parse_duration(s).is_some(), "{what}: {s:?} does not parse");
    }

    fn assert_exact_bytes(s: &str, what: &str) {
        assert!(
            s.ends_with(" B") && parse_bytes(s).is_some(),
            "{what}: {s:?} is not an exact byte count"
        );
    }

    fn assert_display_duration(s: &str, what: &str) {
        let expected = match duration_shape(s) {
            ("ns", d) => d == 0,
            ("µs" | "ms" | "s", d) => d == 2,
            _ => false,
        };
        assert!(expected, "{what}: {s:?} is not in the display format");
    }

    // cargo run -p test-all-features --example basic_all_features --features hotpath-alloc,hotpath-cloud
    #[test]
    fn cloud_report_renders_exact_values() {
        let report = run_with_features("hotpath-alloc,hotpath-cloud");

        let timing = report.functions_timing.expect("functions_timing section");
        assert!(!timing.data.is_empty());
        assert_exact_duration(&timing.time_elapsed, "time_elapsed");
        for entry in &timing.data {
            assert_exact_duration(&entry.avg, &format!("{} avg", entry.name));
            assert_exact_duration(&entry.total, &format!("{} total", entry.name));
            for (key, value) in &entry.percentiles {
                assert_exact_duration(value, &format!("{} {key}", entry.name));
            }
            // The exact string must carry the value the guard summed, not a
            // rounded one: avg * sampled_calls lands on total exactly only
            // when both survived the formatting.
            if entry.sampled_calls == entry.calls && entry.calls > 0 {
                let avg = parse_duration(&entry.avg).unwrap();
                let total = parse_duration(&entry.total).unwrap();
                assert!(
                    total - avg * entry.calls < entry.calls,
                    "{}: total {total} is not avg {avg} * calls {} within integer division",
                    entry.name,
                    entry.calls
                );
            }
        }

        let alloc = report.functions_alloc.expect("functions_alloc section");
        let total_allocated = alloc.total_allocated.as_deref().expect("total_allocated");
        assert!(
            total_allocated.ends_with(" B"),
            "total_allocated {total_allocated:?} is not an exact byte count"
        );
        let sync_entries: Vec<_> = alloc.data.iter().filter(|e| e.avg != "N/A").collect();
        assert!(!sync_entries.is_empty(), "no sync alloc entries in report");
        for entry in sync_entries {
            for (what, value) in [("avg", &entry.avg), ("total", &entry.total)] {
                assert!(
                    value.ends_with(" B"),
                    "{} {what}: {value:?} is not an exact byte count",
                    entry.name
                );
                assert!(
                    parse_bytes(value).is_some(),
                    "{} {what}: {value:?} does not parse",
                    entry.name
                );
            }
        }

        let mutexes = report.mutexes.expect("mutexes section");
        for entry in &mutexes.data {
            assert_exact_duration(&entry.wait_avg, &format!("{} wait_avg", entry.label));
            assert_exact_duration(&entry.acquire_avg, &format!("{} acquire_avg", entry.label));
        }
        let channels = report.channels.expect("channels section");
        for entry in &channels.data {
            let delay_avg = entry.delay_avg.as_deref().expect("delay_avg");
            assert_exact_duration(delay_avg, &format!("{} delay_avg", entry.label));
        }

        let threads = report.threads.expect("threads section");
        for (what, value) in [
            ("rss_bytes", &threads.rss_bytes),
            ("rss_bytes_max", &threads.rss_bytes_max),
            ("total_alloc_bytes", &threads.total_alloc_bytes),
            ("total_dealloc_bytes", &threads.total_dealloc_bytes),
        ] {
            let value = value.as_deref().unwrap_or_else(|| panic!("threads {what}"));
            assert_exact_bytes(value, &format!("threads {what}"));
        }
        let diff = threads
            .alloc_dealloc_diff
            .as_deref()
            .expect("threads alloc_dealloc_diff");
        assert_exact_bytes(diff.trim_start_matches('-'), "threads alloc_dealloc_diff");
        let allocating: Vec<_> = threads
            .data
            .iter()
            .filter(|t| t.alloc_bytes.is_some())
            .collect();
        assert!(!allocating.is_empty(), "no thread with alloc stats");
        for thread in allocating {
            for (what, value) in [
                ("alloc_bytes", &thread.alloc_bytes),
                ("dealloc_bytes", &thread.dealloc_bytes),
            ] {
                let value = value
                    .as_deref()
                    .unwrap_or_else(|| panic!("{} {what}", thread.name));
                assert_exact_bytes(value, &format!("{} {what}", thread.name));
            }
            let mem_diff = thread.mem_diff.as_deref().expect("mem_diff");
            assert_exact_bytes(
                mem_diff.trim_start_matches('-'),
                &format!("{} mem_diff", thread.name),
            );
        }
    }

    // cargo run -p test-all-features --example basic_all_features --features hotpath-alloc
    #[test]
    fn display_format_kept_without_the_cloud_feature() {
        let report = run_with_features("hotpath-alloc");

        let timing = report.functions_timing.expect("functions_timing section");
        assert!(!timing.data.is_empty());
        for entry in &timing.data {
            assert_display_duration(&entry.avg, &format!("{} avg", entry.name));
            assert_display_duration(&entry.total, &format!("{} total", entry.name));
        }

        let alloc = report.functions_alloc.expect("functions_alloc section");
        let big = alloc
            .data
            .iter()
            .find(|e| e.name.ends_with("::main"))
            .expect("main alloc entry");
        assert!(
            big.total.ends_with(" KB") || big.total.ends_with(" MB"),
            "main total {:?} should use a rounded unit in display mode",
            big.total
        );

        let threads = report.threads.expect("threads section");
        let max_rss = threads.rss_bytes_max.as_deref().expect("rss_bytes_max");
        assert!(
            !max_rss.ends_with(" B"),
            "threads rss_bytes_max {max_rss:?} is in the exact format without hotpath-cloud"
        );
    }
}
