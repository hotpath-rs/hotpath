#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use hotpath::json::JsonChannelsList;

    use crate::common::assert_contains_all;
    use crate::common::endpoints::assert_list_and_logs_endpoints;
    use crate::common::example::Example;

    fn example(name: &str) -> Example {
        Example::new("test-channels-std", name)
    }

    fn channels(name: &str) -> JsonChannelsList {
        example(name)
            .report()
            .channels
            .expect("No channels section in report")
    }

    // cargo run -p test-channels-std --example basic_std --features hotpath
    #[test]
    fn test_basic_output() {
        let output = example("basic_std").output();

        assert!(
            output.status.success(),
            "Command failed with status: {}",
            output.status
        );

        assert!(!output.stderr.is_empty(), "Stderr is empty");
        let all_expected = ["Actor 1", "unbounded-channel", "bounded[10]"];

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-std --example basic_json_std --features hotpath
    #[test]
    fn test_basic_json_output() {
        let stdout = example("basic_json_std").stdout();

        let all_expected = ["\"label\": \"unbounded\"", "\"label\": \"bounded\""];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-std --example closed_std --features hotpath
    #[test]
    fn test_closed_channels_output() {
        example("closed_std").stdout();
    }

    // cargo run -p test-channels-std --example iter_std --features hotpath
    #[test]
    fn test_iter_output() {
        let stdout = example("iter_std").stdout();

        let all_expected = [
            "Actor 1",
            "Actor 1-2",
            "Actor 1-3",
            "bounded",
            "bounded-2",
            "bounded-3",
        ];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-std --example slow_consumer_std --features hotpath
    #[test]
    fn test_slow_consumer_no_panic() {
        let stdout = example("slow_consumer_std").stdout();

        assert!(
            stdout.contains("Slow consumer example completed!"),
            "Expected completion message not found.\nOutput:\n{}",
            stdout
        );
    }

    // HOTPATH_METRICS_PORT=6770 TEST_SLEEP_SECONDS=10 cargo run -p test-channels-std --example basic_std --features hotpath
    #[test]
    fn test_data_endpoints() {
        assert_list_and_logs_endpoints(
            example("basic_std"),
            6770,
            "channels",
            &["basic_std.rs", "unbounded-channel", "Actor 1"],
            |channels: &JsonChannelsList| channels.data.first().and_then(|channel| channel.id),
        );
    }

    // Bounded std requires `capacity`, which the macro accepts in any argument
    // position. Every order of `capacity`/`label`/`log` must compile and register a channel.
    //
    // cargo run -p test-channels-std --example wrap_arg_orders_std --features hotpath
    #[test]
    fn test_capacity_arg_orders() {
        let channels = channels("wrap_arg_orders_std");

        for label in ["a", "b", "c", "d", "e", "f", "g", "h"] {
            channels
                .data
                .iter()
                .find(|c| c.label == label)
                .unwrap_or_else(|| panic!("channel {label:?} not found"));
        }
    }

    // The self-tracked queue counter reports the exact depth (50 messages parked,
    // none received), where a forwarder would drain immediately and report ~0.
    //
    // cargo run -p test-channels-std --example wrap_std --features hotpath
    #[test]
    fn test_exact_queue_depth() {
        let channels = channels("wrap_std");

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
    // cargo run -p test-channels-std --example wrap_unbounded_std --features hotpath
    #[test]
    fn test_unbounded_sent_received() {
        let channels = channels("wrap_unbounded_std");

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
    // cargo run -p test-channels-std --example wrap_concurrent_std --features hotpath
    #[test]
    fn test_concurrent_no_underflow() {
        let channels = channels("wrap_concurrent_std");

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
    // closed. std receivers are not Clone, so there is no clone-count path.
    //
    // cargo run -p test-channels-std --example wrap_closed_std --features hotpath
    #[test]
    fn test_receiver_dropped_closes() {
        let channels = channels("wrap_closed_std");

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
}
