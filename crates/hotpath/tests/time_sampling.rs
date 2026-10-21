#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use hotpath::json::{JsonFunctionEntry, JsonReport};

    use crate::common::example::Example;

    /// Events per resource in the example workload.
    const CALLS: u64 = 1000;

    /// Runs the time-sampling example and parses the JSON report. Clears
    /// sampling env vars first so the host environment cannot leak in.
    fn run_example_with_features(features: &str, envs: &[(&str, &str)]) -> JsonReport {
        let mut example = Example::new("test-all-features", "time_sampling").features(features);
        for name in [
            "HOTPATH_TIME_SAMPLING_RATE",
            "HOTPATH_FUNCTIONS_TIME_SAMPLING_RATE",
            "HOTPATH_MUTEXES_TIME_SAMPLING_RATE",
            "HOTPATH_RW_LOCKS_TIME_SAMPLING_RATE",
            "HOTPATH_FUTURES_TIME_SAMPLING_RATE",
            "HOTPATH_CHANNELS_TIME_SAMPLING_RATE",
        ] {
            example = example.env_remove(name);
        }
        example.json().envs(envs).report()
    }

    fn run_example(envs: &[(&str, &str)]) -> JsonReport {
        run_example_with_features("hotpath", envs)
    }

    /// Sampling is random: the timed share must land within six standard
    /// deviations of `rate * total`.
    fn assert_sampled_near(label: &str, sampled: u64, total: u64, rate: f64) {
        let expected = rate * total as f64;
        let tolerance = 6.0 * (total as f64 * rate * (1.0 - rate)).sqrt();
        let diff = (sampled as f64 - expected).abs();
        assert!(
            diff <= tolerance,
            "{label}: sampled {sampled} of {total} at rate {rate}, expected {expected:.0} +/- {tolerance:.0}"
        );
    }

    fn function_entry<'a>(report: &'a JsonReport, name: &str) -> &'a JsonFunctionEntry {
        report
            .functions_timing
            .as_ref()
            .expect("No functions_timing section")
            .data
            .iter()
            .find(|f| f.name.contains(name))
            .unwrap_or_else(|| panic!("No `{name}` entry in functions_timing"))
    }

    // cargo run -p test-all-features --example time_sampling --features hotpath (json)
    #[test]
    fn test_no_sampling_is_passthrough() {
        let report = run_example(&[]);

        assert!(report.time_sampling.is_none());
        let work = function_entry(&report, "work_a");
        assert_eq!(work.calls, CALLS);
        assert_eq!(work.sampled_calls, CALLS);

        let mutex = &report.mutexes.as_ref().expect("No mutexes section").data[0];
        assert_eq!(mutex.count, CALLS);
        assert_eq!(mutex.sampled_count, CALLS);

        let channel = &report.channels.as_ref().expect("No channels section").data[0];
        assert_eq!(channel.received_count, CALLS);
        assert_eq!(channel.delay_sampled_count, Some(CALLS));
    }

    #[test]
    fn test_global_rate_applies_to_all_resources() {
        let report = run_example(&[("HOTPATH_TIME_SAMPLING_RATE", "0.5")]);

        let rates = report.time_sampling.as_ref().expect("No time_sampling");
        for resource in ["functions", "mutexes", "rw_locks", "futures", "channels"] {
            assert_eq!(rates.get(resource), Some(&0.5), "rate for {resource}");
        }

        // `work_a` and `work_b` alternate on one thread; both must get their share.
        for name in ["work_a", "work_b"] {
            let work = function_entry(&report, name);
            assert_eq!(work.calls, CALLS);
            assert_sampled_near(name, work.sampled_calls, work.calls, 0.5);
        }

        // The wrapper guard is exempt from sampling.
        let main = function_entry(&report, "main");
        assert_eq!(main.sampled_calls, main.calls);

        let mutex = &report.mutexes.as_ref().expect("No mutexes section").data[0];
        assert_eq!(mutex.count, CALLS);
        assert_sampled_near("mutex", mutex.sampled_count, mutex.count, 0.5);

        let rw = &report.rw_locks.as_ref().expect("No rw_locks section").data[0];
        assert_eq!(rw.read_count, CALLS);
        assert_sampled_near("rw read", rw.read_sampled_count, rw.read_count, 0.5);
        assert_eq!(rw.write_count, CALLS / 2);
        assert_sampled_near("rw write", rw.write_sampled_count, rw.write_count, 0.5);

        let channel = &report.channels.as_ref().expect("No channels section").data[0];
        assert_eq!(channel.sent_count, CALLS);
        assert_eq!(channel.received_count, CALLS);
        let delay_sampled = channel.delay_sampled_count.expect("No delay_sampled_count");
        assert_sampled_near("channel", delay_sampled, channel.received_count, 0.5);
        assert_ne!(channel.delay_avg.as_deref(), Some("-"));
    }

    #[test]
    fn test_rate_zero_is_count_only() {
        let report = run_example(&[("HOTPATH_TIME_SAMPLING_RATE", "0")]);

        let work = function_entry(&report, "work_a");
        assert_eq!(work.calls, CALLS);
        assert_eq!(work.sampled_calls, 0);
        assert_eq!(work.avg, "-");
        assert_eq!(work.total, "-");
        assert_eq!(work.percent_total, "-");

        // The wrapper guard stays measured even in count-only mode.
        let main = function_entry(&report, "main");
        assert_eq!(main.calls, 1);
        assert_eq!(main.sampled_calls, 1);

        let mutex = &report.mutexes.as_ref().expect("No mutexes section").data[0];
        assert_eq!(mutex.count, CALLS);
        assert_eq!(mutex.sampled_count, 0);
        assert_eq!(mutex.wait_avg, "-");

        let channel = &report.channels.as_ref().expect("No channels section").data[0];
        assert_eq!(channel.received_count, CALLS);
        assert_eq!(channel.delay_sampled_count, Some(0));
        assert_eq!(channel.delay_avg.as_deref(), Some("-"));
    }

    #[test]
    fn test_per_resource_env_beats_global() {
        let report = run_example(&[
            ("HOTPATH_TIME_SAMPLING_RATE", "0"),
            ("HOTPATH_FUNCTIONS_TIME_SAMPLING_RATE", "1.0"),
        ]);

        let work = function_entry(&report, "work_a");
        assert_eq!(work.sampled_calls, CALLS);

        let mutex = &report.mutexes.as_ref().expect("No mutexes section").data[0];
        assert_eq!(mutex.sampled_count, 0);
    }

    #[test]
    fn test_builder_rate_applies() {
        let report = run_example(&[("TEST_BUILDER_TIME_SAMPLING_RATE", "0.5")]);

        let work = function_entry(&report, "work_a");
        assert_eq!(work.calls, CALLS);
        assert_sampled_near("work_a", work.sampled_calls, work.calls, 0.5);

        let mutex = &report.mutexes.as_ref().expect("No mutexes section").data[0];
        assert_sampled_near("mutex", mutex.sampled_count, mutex.count, 0.5);
    }

    #[test]
    fn test_env_beats_builder() {
        let report = run_example(&[
            ("TEST_BUILDER_FUNCTIONS_TIME_SAMPLING_RATE", "0"),
            ("HOTPATH_FUNCTIONS_TIME_SAMPLING_RATE", "1.0"),
        ]);

        let work = function_entry(&report, "work_a");
        assert_eq!(work.sampled_calls, CALLS);
    }

    // Under hotpath-alloc the guards measure both allocations and time:
    // durations respect the rate while allocation metrics stay exact.
    #[test]
    fn test_alloc_mode_samples_durations_only() {
        let report =
            run_example_with_features("hotpath-alloc", &[("HOTPATH_TIME_SAMPLING_RATE", "0.5")]);

        let rates = report.time_sampling.as_ref().expect("No time_sampling");
        assert_eq!(rates.get("functions"), Some(&0.5));

        let work = function_entry(&report, "work_a");
        assert_eq!(work.calls, CALLS);
        assert_sampled_near("work_a", work.sampled_calls, work.calls, 0.5);
        assert_ne!(work.avg, "-");

        let alloc_work = report
            .functions_alloc
            .as_ref()
            .expect("No functions_alloc section")
            .data
            .iter()
            .find(|f| f.name.contains("work_a"))
            .expect("No `work_a` entry in functions_alloc");
        assert_eq!(alloc_work.calls, CALLS);
        assert_eq!(alloc_work.sampled_calls, CALLS);
    }

    // Poll counts vary with runtime scheduling, so only count-only mode
    // (rate 0) gives a deterministic futures assertion.
    // cargo run -p test-futures --example basic_futures --features hotpath (json)
    #[test]
    fn test_futures_count_only() {
        let report = Example::new("test-futures", "basic_futures")
            .json()
            .env("HOTPATH_FUTURES_TIME_SAMPLING_RATE", "0")
            .env("HOTPATH_REPORT", "futures")
            .report();

        let futures = report.futures.expect("No futures section");
        assert!(futures.data.iter().any(|e| e.total_polls > 0));
        for entry in &futures.data {
            assert_eq!(entry.sampled_polls, 0, "sampled polls for {}", entry.label);
            assert_eq!(
                entry.total_poll_duration_ns, 0,
                "duration recorded for {}",
                entry.label
            );
        }
    }
}
