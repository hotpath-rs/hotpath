#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use hotpath::json::JsonPmuCounters;

    use crate::common::example::Example;

    // HOTPATH_OUTPUT_FORMAT=json cargo run -p test-tokio-async --example pmu_counters --features hotpath,hotpath-pmu
    #[test]
    fn test_pmu_counters_report() {
        let report = Example::new("test-tokio-async", "pmu_counters")
            .features("hotpath,hotpath-pmu")
            .env("HOTPATH_OUTPUT_FORMAT", "json")
            .report();
        let pmu = report
            .threads
            .expect("No threads section in report")
            .pmu
            .expect("No pmu counters in threads section");

        // Printed so CI logs show which counters the runner exposes.
        eprintln!("{}", serde_json::to_string_pretty(&pmu).unwrap());

        assert_counters(pmu);
    }

    #[cfg(not(target_os = "linux"))]
    fn assert_counters(pmu: JsonPmuCounters) {
        assert!(
            matches!(pmu, JsonPmuCounters::Unavailable { .. }),
            "pmu is Linux-only"
        );
    }

    #[cfg(target_os = "linux")]
    fn assert_counters(pmu: JsonPmuCounters) {
        use hotpath::json::{JsonPmuValue, PmuEvent};

        let events = match pmu {
            JsonPmuCounters::Available { events } => events,
            // CI lowers kernel.perf_event_paranoid; a dev box may not have.
            JsonPmuCounters::Unavailable { reason } if std::env::var_os("CI").is_none() => {
                eprintln!("skipping, perf counters unavailable: {reason}");
                return;
            }
            JsonPmuCounters::Unavailable { reason } => {
                panic!("perf counters unavailable on Linux CI: {reason}")
            }
        };

        for expected in PmuEvent::ALL {
            let entry = events
                .iter()
                .find(|e| e.event == expected)
                .unwrap_or_else(|| panic!("missing event {expected:?}"));
            let JsonPmuValue::Counted {
                process,
                application,
                profiler,
                main_thread,
                ..
            } = entry.value
            else {
                continue;
            };
            assert!(
                application <= process && profiler <= process,
                "{expected:?}: application {application} and profiler {profiler} must not exceed process {process}"
            );
            // Two threads each run a 20M-iteration loop, half of it on the
            // guard's thread.
            if expected == PmuEvent::Instructions {
                assert!(application >= 40_000_000, "instructions: {application}");
                let main_thread = main_thread.expect("main thread instructions");
                assert!(main_thread >= 20_000_000, "main thread: {main_thread}");
                assert!(main_thread < application, "main thread: {main_thread}");
            }
        }
    }
}
