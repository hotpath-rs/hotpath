#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use std::process::Command;

    use hotpath::json::JsonChannelsList;

    use crate::common::endpoints::assert_list_and_logs_endpoints;
    use crate::common::example::Example;
    use crate::common::{assert_contains_all, path_sep};

    fn example(name: &str) -> Example {
        Example::new("test-channels-tokio", name)
    }

    fn channels(name: &str) -> JsonChannelsList {
        example(name)
            .report()
            .channels
            .expect("No channels section in report")
    }

    // cargo run -p test-channels-tokio --example agg_tokio --features hotpath
    #[test]
    fn test_default_mode_aggregates_per_callsite() {
        let channels = channels("agg_tokio");

        assert_eq!(
            channels.data.len(),
            1,
            "default mode must aggregate all loop instances into one entry, got: {:?}",
            channels.data
        );
        let entry = &channels.data[0];
        assert_eq!(entry.instances, 5, "5 channels created at the call site");
        assert_eq!(entry.closed_instances, 5, "all endpoints dropped");
        assert_eq!(entry.state, None, "aggregated entries report no state");
        assert_eq!(entry.sent_count, 10, "summed across instances");
        assert_eq!(entry.received_count, 10, "summed across instances");
        assert_eq!(entry.iter, 0, "aggregated entries carry no iter suffix");

        // The rate window is at most the report's total elapsed time, so the rate
        // is bounded below by count / total elapsed.
        let rate = entry
            .sent_per_sec
            .expect("aggregated entry must report a rate");
        let elapsed_secs = channels.current_elapsed_ns as f64 / 1e9;
        let floor = entry.sent_count as f64 / elapsed_secs;
        assert!(
            rate >= floor * 0.9,
            "rate {rate} inconsistent with count over elapsed ({floor})"
        );
    }

    // cargo run -p test-channels-tokio --example agg_queue_tokio --features hotpath
    #[test]
    fn test_aggregated_queue_depth_is_combined() {
        let channels = channels("agg_queue_tokio");

        assert_eq!(channels.data.len(), 1);
        let entry = &channels.data[0];
        assert_eq!(entry.instances, 2);
        // Two live instances each hold 3 messages: combined depth, not the
        // largest single-instance snapshot (3).
        assert_eq!(entry.queue_size, Some(6), "combined in-flight depth");
        assert!(
            entry.queue_size <= entry.max_queue_size,
            "current depth must stay within the tracked peak: {:?} > {:?}",
            entry.queue_size,
            entry.max_queue_size
        );
        assert_eq!(entry.state, None, "aggregated entries report no state");
    }

    // cargo run -p test-channels-tokio --example agg_many_tokio --features hotpath
    #[test]
    fn test_default_mode_state_stays_bounded() {
        let channels = channels("agg_many_tokio");

        // Boundedness: thousands of default-mode channels at one call site
        // must not register per-instance entries.
        assert_eq!(
            channels.data.len(),
            1,
            "expected a single aggregated entry for 2000 instances"
        );
        let entry = &channels.data[0];
        assert_eq!(entry.instances, 2000);
        assert_eq!(entry.state, None);
    }

    // cargo build -p test-channels-tokio --example iter_tokio
    #[test]
    fn test_iter_param_compiles_without_feature() {
        let output = Command::new("cargo")
            .args([
                "build",
                "-p",
                "test-channels-tokio",
                "--example",
                "iter_tokio",
            ])
            .output()
            .expect("Failed to execute command");

        assert!(
            output.status.success(),
            "feature-off build of `channel!(..., iter = true)` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // cargo run -p test-channels-tokio --example basic_tokio --features hotpath
    #[test]
    fn test_basic_output() {
        let output = example("basic_tokio").output();

        assert!(
            output.status.success(),
            "Command failed with status: {}",
            output.status
        );

        assert!(!output.stderr.is_empty(), "Stderr is empty");
        let all_expected = [
            "Actor 1",
            "bounded-channel",
            "hello-there",
            "unbounded",
            "bounded[10]",
            "oneshot",
        ];

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-tokio --example basic_json_tokio --features hotpath
    #[test]
    fn test_basic_json_output() {
        let stdout = example("basic_json_tokio").stdout();

        let sep = path_sep();
        let json_path = format!("\"label\": \"examples{sep}basic_json_tokio.rs:");
        let all_expected = [
            json_path.as_str(),
            "\"label\": \"hello-there\"",
            "\"state\": \"notified\"",
        ];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-tokio --example closed_tokio --features hotpath
    #[test]
    fn test_closed_channels_output() {
        example("closed_tokio").stdout();
    }

    // cargo run -p test-channels-tokio --example oneshot_closed_tokio --features hotpath
    #[test]
    fn test_oneshot_closed_output() {
        let stdout = example("oneshot_closed_tokio").stdout();

        let all_expected = ["oneshot_closed_tokio.rs:"];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-tokio --example iter_tokio --features hotpath
    #[test]
    fn test_iter_output() {
        let stdout = example("iter_tokio").stdout();

        let sep = path_sep();
        let iter_41 = format!("examples{sep}iter_tokio.rs:41");
        let iter_41_2 = format!("examples{sep}iter_tokio.rs:41-2");
        let iter_41_3 = format!("examples{sep}iter_tokio.rs:41-3");
        let iter_53 = format!("examples{sep}iter_tokio.rs:53");
        let iter_53_2 = format!("examples{sep}iter_tokio.rs:53-2");
        let iter_53_3 = format!("examples{sep}iter_tokio.rs:53-3");
        let all_expected = [
            "Actor 1",
            "Actor 1-2",
            "Actor 1-3",
            iter_41.as_str(),
            iter_41_2.as_str(),
            iter_41_3.as_str(),
            iter_53.as_str(),
            iter_53_2.as_str(),
            iter_53_3.as_str(),
        ];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-tokio --example slow_consumer_tokio --features hotpath
    #[test]
    fn test_slow_consumer_no_panic() {
        let stdout = example("slow_consumer_tokio").stdout();

        assert!(
            stdout.contains("Slow consumer example completed!"),
            "Expected completion message not found.\nOutput:\n{}",
            stdout
        );
    }

    // cargo run -p test-channels-tokio --example guard_timeout_channels --features hotpath
    #[test]
    fn test_guard_timeout_output() {
        let stdout = example("guard_timeout_channels").stdout();
        let expected_content = ["[hotpath]", "| channels", "timeout-channel"];

        assert_contains_all(&stdout, &expected_content);
    }

    // HOTPATH_METRICS_PORT=6773 TEST_SLEEP_SECONDS=10 cargo run -p test-channels-tokio --example basic_tokio --features hotpath
    #[test]
    fn test_data_endpoints() {
        assert_list_and_logs_endpoints(
            example("basic_tokio"),
            6773,
            "channels",
            &["basic_tokio.rs", "bounded-channel", "Actor 1"],
            |channels: &JsonChannelsList| channels.data.first().and_then(|channel| channel.id),
        );
    }

    // HOTPATH_OUTPUT_FORMAT=none cargo run -p test-channels-tokio --example basic_tokio --features hotpath
    #[test]
    fn test_format_none_suppresses_output() {
        let stdout = example("basic_tokio")
            .env("HOTPATH_OUTPUT_FORMAT", "none")
            .stdout();

        assert!(
            stdout.contains("Example completed!"),
            "Application output should still be present.\nGot:\n{stdout}"
        );

        let not_expected = [
            "[hotpath]",
            "bounded-channel",
            "hello-there",
            "Channel throughput",
        ];

        for not_exp in not_expected {
            assert!(
                !stdout.contains(not_exp),
                "Channel output should be suppressed with HOTPATH_OUTPUT_FORMAT=none.\nFound: {not_exp}\nGot:\n{stdout}"
            );
        }
    }

    // cargo run -p test-channels-tokio --example channels_file_output --features hotpath
    #[test]
    fn test_channels_file_output() {
        use std::fs;
        use std::path::Path;

        let output_path = "tmp/channels_output_test.json";

        fs::create_dir_all("tmp").ok();
        if Path::new(output_path).exists() {
            fs::remove_file(output_path).ok();
        }

        example("channels_file_output").stdout();

        assert!(
            Path::new(output_path).exists(),
            "Output file was not created at {}",
            output_path
        );

        let file_content = fs::read_to_string(output_path).expect("Failed to read output file");

        let expected_content = ["test-channel", "\"sent_count\"", "\"received_count\""];

        assert_contains_all(&file_content, &expected_content);

        fs::remove_file(output_path).ok();
    }

    // The self-tracked queue counter reports the exact depth (50 messages parked,
    // none received), where a forwarder would drain immediately and report ~0.
    // Tokio recovers bounded capacity from `max_capacity()`, so no `capacity` arg.
    //
    // cargo run -p test-channels-tokio --example wrap_tokio --features hotpath
    #[test]
    fn test_exact_queue_depth() {
        let channels = channels("wrap_tokio");

        let entry = channels
            .data
            .iter()
            .find(|c| c.label == "wrap-queue")
            .expect("wrap-queue channel not found");
        assert_eq!(entry.sent_count, 50, "expected 50 sends");
        assert_eq!(
            entry.received_count, 0,
            "expected 0 receives at report time"
        );
        assert_eq!(
            entry.queue_size,
            Some(50),
            "expected exact queue depth of 50"
        );
        assert_eq!(
            entry.max_queue_size,
            Some(50),
            "expected max queue depth of 50"
        );
    }

    // Unbounded: every message sent and drained, queue back to zero with the
    // high-water mark preserved.
    //
    // cargo run -p test-channels-tokio --example wrap_unbounded_tokio --features hotpath
    #[test]
    fn test_unbounded_sent_received() {
        let channels = channels("wrap_unbounded_tokio");

        let entry = channels
            .data
            .iter()
            .find(|c| c.label == "wrap-unbounded")
            .expect("wrap-unbounded channel not found");
        assert_eq!(entry.sent_count, 200, "expected 200 sends");
        assert_eq!(entry.received_count, 200, "expected 200 receives");
        assert_eq!(entry.queue_size, Some(0), "expected drained queue");
        assert_eq!(
            entry.max_queue_size,
            Some(200),
            "expected max queue depth of 200"
        );
    }

    // A producer racing a consumer must never underflow the depth counter: debug
    // builds would panic the consumer (caught by `channels`), release builds
    // would wrap to an absurd queue length.
    //
    // cargo run -p test-channels-tokio --example wrap_concurrent_tokio --features hotpath
    #[test]
    fn test_concurrent_no_underflow() {
        let channels = channels("wrap_concurrent_tokio");

        let entry = channels
            .data
            .iter()
            .find(|c| c.label == "wrap-concurrent")
            .expect("wrap-concurrent channel not found");
        assert!(
            entry.received_count <= entry.sent_count,
            "received ({}) must not exceed sent ({})",
            entry.received_count,
            entry.sent_count
        );
        assert!(
            entry.max_queue_size.unwrap_or(0) <= entry.sent_count as usize,
            "max queue ({:?}) is absurd - the depth counter underflowed and wrapped",
            entry.max_queue_size
        );
    }

    // Dropping the single receiver while the sender is alive must mark the channel
    // closed. tokio receivers are not Clone, so there is no clone-count path.
    //
    // cargo run -p test-channels-tokio --example wrap_closed_tokio --features hotpath
    #[test]
    fn test_receiver_dropped_closes() {
        let channels = channels("wrap_closed_tokio");

        let entry = channels
            .data
            .iter()
            .find(|c| c.label == "recv-dropped")
            .expect("recv-dropped channel not found");
        assert_eq!(
            entry.state.as_deref(),
            Some("closed"),
            "expected closed state after receiver drop"
        );
    }

    // Weak senders: the example asserts the downgrade/upgrade lifecycle in-process;
    // here the report must see the upgraded sender's traffic and exactly one Closed
    // transition.
    //
    // cargo run -p test-channels-tokio --example weak_tokio --features hotpath
    #[test]
    fn test_weak_senders() {
        let channels = channels("weak_tokio");

        for label in ["wrap-weak", "wrap-weak-unbounded"] {
            let entry = channels
                .data
                .iter()
                .find(|c| c.label == label)
                .unwrap_or_else(|| panic!("{label} channel not found"));
            assert_eq!(entry.sent_count, 2, "expected 2 sends on {label}");
            assert_eq!(entry.received_count, 2, "expected 2 receives on {label}");
            assert_eq!(
                entry.state.as_deref(),
                Some("closed"),
                "expected closed state after all strong senders dropped on {label}"
            );
        }
    }

    // Batch receive: recv_many drains in chunks but every message gets its own
    // receive event, so counts are exact, the queue returns to zero, and the delay
    // histogram is populated.
    //
    // cargo run -p test-channels-tokio --example recv_many_tokio --features hotpath
    #[test]
    fn test_recv_many() {
        let channels = channels("recv_many_tokio");

        let entry = channels
            .data
            .iter()
            .find(|c| c.label == "wrap-recv-many")
            .expect("wrap-recv-many channel not found");
        assert_eq!(entry.sent_count, 60, "expected 60 sends");
        assert_eq!(entry.received_count, 60, "expected 60 receives");
        assert_eq!(entry.queue_size, Some(0), "expected drained queue");
        assert!(
            entry.delay_avg.is_some(),
            "expected populated delay histogram"
        );

        let unbounded = channels
            .data
            .iter()
            .find(|c| c.label == "wrap-recv-many-unbounded")
            .expect("wrap-recv-many-unbounded channel not found");
        assert_eq!(unbounded.sent_count, 30, "expected 30 sends");
        assert_eq!(unbounded.received_count, 30, "expected 30 receives");
        assert_eq!(unbounded.queue_size, Some(0), "expected drained queue");
    }

    // Blocking variants driven from std threads without a runtime: stats recorded,
    // no panic (the example itself asserts message ordering).
    //
    // cargo run -p test-channels-tokio --example blocking_tokio --features hotpath
    #[test]
    fn test_blocking_off_runtime() {
        let channels = channels("blocking_tokio");

        let entry = channels
            .data
            .iter()
            .find(|c| c.label == "wrap-blocking")
            .expect("wrap-blocking channel not found");
        assert_eq!(entry.sent_count, 25, "expected 25 sends");
        assert_eq!(entry.received_count, 25, "expected 25 receives");
        assert_eq!(entry.queue_size, Some(0), "expected drained queue");
    }

    // A timed-out send_timeout on a full channel rolls the depth counter back:
    // the failed send is not counted and the queue never exceeds capacity.
    //
    // cargo run -p test-channels-tokio --example send_timeout_tokio --features hotpath
    #[test]
    fn test_send_timeout_rollback() {
        let channels = channels("send_timeout_tokio");

        let entry = channels
            .data
            .iter()
            .find(|c| c.label == "wrap-send-timeout")
            .expect("wrap-send-timeout channel not found");
        assert_eq!(entry.sent_count, 5, "timed-out send must not be counted");
        assert_eq!(
            entry.queue_size,
            Some(5),
            "queue must not exceed capacity after rollback"
        );
        assert_eq!(
            entry.max_queue_size,
            Some(5),
            "max queue must not exceed capacity after rollback"
        );
    }

    // Manual polling via poll_recv/poll_recv_many records every receive; the
    // example exercises a Pending-then-Ready sequence on the reusable scratch buffer.
    //
    // cargo run -p test-channels-tokio --example poll_recv_tokio --features hotpath
    #[test]
    fn test_poll_recv() {
        let channels = channels("poll_recv_tokio");

        let entry = channels
            .data
            .iter()
            .find(|c| c.label == "wrap-poll-recv")
            .expect("wrap-poll-recv channel not found");
        assert_eq!(entry.sent_count, 30, "expected 30 sends");
        assert_eq!(entry.received_count, 30, "expected 30 receives");
        assert_eq!(entry.queue_size, Some(0), "expected drained queue");
    }

    // Two `channel!` invocations on one physical line (same message type) must
    // register distinct entries: the registration key includes the column, so
    // the second call site does not reuse the first one's id. The displayed
    // source keeps the plain `file:line` form.
    //
    // cargo run -p test-channels-tokio --example same_line_tokio --features hotpath
    #[test]
    fn test_same_line_call_sites_stay_distinct() {
        let channels = channels("same_line_tokio");

        let a = channels
            .data
            .iter()
            .find(|c| c.label == "same-line-a")
            .expect("same-line-a channel not found");
        let b = channels
            .data
            .iter()
            .find(|c| c.label == "same-line-b")
            .expect("same-line-b channel not found");

        assert_ne!(a.id, b.id, "same-line call sites must not share an entry");
        assert_eq!(a.sent_count, 3, "counts must not merge across call sites");
        assert_eq!(b.sent_count, 5, "counts must not merge across call sites");
        assert_eq!(a.channel_type, "bounded[4]");
        assert_eq!(b.channel_type, "bounded[8]");
        assert_eq!(a.instances, 1);
        assert_eq!(b.instances, 1);

        // The displayed source is identical for both (same file:line) and the
        // path contains no ':', so exactly one colon proves the column stays
        // out of the display string.
        assert_eq!(a.source, b.source, "one physical line renders one source");
        assert_eq!(
            a.source.matches(':').count(),
            1,
            "source must stay file:line"
        );
    }
}
