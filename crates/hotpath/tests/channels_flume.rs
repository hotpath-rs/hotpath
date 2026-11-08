#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use hotpath::json::JsonChannelsList;

    use crate::common::endpoints::assert_list_and_logs_endpoints;
    use crate::common::example::Example;
    use crate::common::{assert_contains_all, path_sep};

    fn example(name: &str) -> Example {
        Example::new("test-channels-flume", name)
    }

    fn channels(name: &str) -> JsonChannelsList {
        example(name)
            .report()
            .channels
            .expect("No channels section in report")
    }

    // cargo run -p test-channels-flume --example basic_flume --features hotpath
    #[test]
    fn test_basic_output() {
        let output = example("basic_flume").output();

        assert!(
            output.status.success(),
            "Command failed with status: {}",
            output.status
        );

        assert!(!output.stderr.is_empty(), "Stderr is empty");
        let all_expected = ["Actor 1", "bounded-channel", "unbounded", "bounded[10]"];

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-flume --example basic_json_flume --features hotpath
    #[test]
    fn test_basic_json_output() {
        let stdout = example("basic_json_flume").stdout();

        let all_expected = ["\"label\": \"unbounded\"", "\"label\": \"bounded\""];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-flume --example closed_flume --features hotpath
    #[test]
    fn test_closed_channels_output() {
        example("closed_flume").stdout();
    }

    // cargo run -p test-channels-flume --example iter_flume --features hotpath
    #[test]
    fn test_iter_output() {
        let stdout = example("iter_flume").stdout();

        let sep = path_sep();
        let iter_27 = format!("examples{sep}iter_flume.rs:27");
        let iter_27_2 = format!("examples{sep}iter_flume.rs:27-2");
        let iter_27_3 = format!("examples{sep}iter_flume.rs:27-3");
        let iter_40 = format!("examples{sep}iter_flume.rs:40");
        let iter_40_2 = format!("examples{sep}iter_flume.rs:40-2");
        let iter_40_3 = format!("examples{sep}iter_flume.rs:40-3");
        let all_expected = [
            iter_27.as_str(),
            iter_27_2.as_str(),
            iter_27_3.as_str(),
            iter_40.as_str(),
            iter_40_2.as_str(),
            iter_40_3.as_str(),
        ];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-flume --example slow_consumer_flume --features hotpath
    #[test]
    fn test_slow_consumer_no_panic() {
        let stdout = example("slow_consumer_flume").stdout();

        assert!(
            stdout.contains("Slow consumer example completed!"),
            "Expected completion message not found.\nOutput:\n{}",
            stdout
        );
    }

    // HOTPATH_METRICS_PORT=6772 TEST_SLEEP_SECONDS=10 cargo run -p test-channels-flume --example basic_flume --features hotpath
    #[test]
    fn test_data_endpoints() {
        assert_list_and_logs_endpoints(
            example("basic_flume"),
            6772,
            "channels",
            &["basic_flume.rs", "bounded-channel", "Actor 1"],
            |channels: &JsonChannelsList| channels.data.first().and_then(|channel| channel.id),
        );
    }

    // The endpoint wrapper samples the real channel length, so it reports the exact
    // depth (50 parked, none received) where a forwarder would report ~0.
    //
    // cargo run -p test-channels-flume --example wrap_flume --features hotpath
    #[test]
    fn test_exact_queue_depth() {
        let channels = channels("wrap_flume");

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

    // Dropping the receiver while the sender is alive must mark the channel closed.
    //
    // cargo run -p test-channels-flume --example wrap_closed_flume --features hotpath
    #[test]
    fn test_receiver_dropped_closes() {
        let channels = channels("wrap_closed_flume");

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

    // The last Receiver clone is dropped while Sender clones are still alive, and the
    // report is taken before any Sender is dropped. The endpoint wrapper must still
    // mark the channel closed.
    //
    // cargo run -p test-channels-flume --example wrap_recv_clone_closed_flume --features hotpath
    #[test]
    fn test_receiver_clone_dropped_closes_with_sender_alive() {
        let channels = channels("wrap_recv_clone_closed_flume");

        let entry = channels
            .data
            .iter()
            .find(|c| c.label == "recv-clone-dropped")
            .expect("recv-clone-dropped channel not found");
        assert_eq!(
            entry.state.as_deref(),
            Some("closed"),
            "expected closed state after all receivers dropped while senders alive"
        );
    }

    // cargo run -p test-channels-flume --example wrap_latency_flume --features hotpath
    #[test]
    fn test_processing_histogram() {
        let channels = channels("wrap_latency_flume");

        assert_eq!(channels.percentiles, vec![50.0, 95.0]);

        // Channels carry an exact send->receive latency histogram in the JSON report.
        let latency = channels
            .data
            .iter()
            .find(|c| c.label == "wrap-latency")
            .expect("wrap-latency channel not found");
        let delay_avg = latency
            .delay_avg
            .as_deref()
            .expect("channel should report delay_avg in JSON");
        assert!(!delay_avg.is_empty(), "delay_avg should not be empty");
        assert_ne!(
            delay_avg, "0ns",
            "expected non-zero send->receive latency (~20ms held in channel)"
        );
        assert!(
            latency.delay_percentiles.contains_key("p50"),
            "expected p50 latency percentile in JSON, got {:?}",
            latency.delay_percentiles
        );
        assert!(
            latency.delay_percentiles.contains_key("p95"),
            "expected p95 latency percentile in JSON, got {:?}",
            latency.delay_percentiles
        );
    }
}
