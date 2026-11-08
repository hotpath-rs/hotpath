#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use hotpath::json::JsonChannelsList;

    use crate::common::endpoints::assert_list_and_logs_endpoints;
    use crate::common::example::Example;
    use crate::common::{assert_contains_all, path_sep};

    fn example(name: &str) -> Example {
        Example::new("test-channels-ftc", name)
    }

    // cargo run -p test-channels-ftc --example basic_ftc --features hotpath
    #[test]
    fn test_basic_output() {
        let output = example("basic_ftc").output();

        assert!(
            output.status.success(),
            "Command failed with status: {}",
            output.status
        );

        assert!(!output.stderr.is_empty(), "Stderr is empty");
        let all_expected = [
            "Actor 1",
            "bounded-channel",
            "oneshot-labeled",
            "bounded[10]",
        ];

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-ftc --example basic_json_ftc --features hotpath
    #[test]
    fn test_basic_json_output() {
        let stdout = example("basic_json_ftc").stdout();

        let all_expected = [
            "\"label\": \"unbounded\"",
            "\"label\": \"bounded\"",
            "\"label\": \"oneshot\"",
            "\"state\": \"notified\"",
        ];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-ftc --example closed_ftc --features hotpath
    #[test]
    fn test_closed_channels_output() {
        example("closed_ftc").stdout();
    }

    // cargo run -p test-channels-ftc --example oneshot_closed_ftc --features hotpath
    #[test]
    fn test_oneshot_closed_output() {
        let stdout = example("oneshot_closed_ftc").stdout();

        let all_expected = ["oneshot-closed"];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-ftc --example iter_ftc --features hotpath
    #[test]
    fn test_iter_output() {
        let stdout = example("iter_ftc").stdout();

        let sep = path_sep();
        let iter_63 = format!("examples{sep}iter_ftc.rs:63");
        let iter_63_2 = format!("examples{sep}iter_ftc.rs:63-2");
        let iter_63_3 = format!("examples{sep}iter_ftc.rs:63-3");
        let all_expected = [
            "Actor 1",
            "Actor 1-2",
            "Actor 1-3",
            "bounded",
            "bounded-2",
            "bounded-3",
            iter_63.as_str(),
            iter_63_2.as_str(),
            iter_63_3.as_str(),
        ];

        assert_contains_all(&stdout, &all_expected);
    }

    // cargo run -p test-channels-ftc --example slow_consumer_ftc --features hotpath
    #[test]
    fn test_slow_consumer_no_panic() {
        let stdout = example("slow_consumer_ftc").stdout();

        assert!(
            stdout.contains("Slow consumer example completed!"),
            "Expected completion message not found.\nOutput:\n{}",
            stdout
        );
    }

    // HOTPATH_METRICS_PORT=6772 TEST_SLEEP_SECONDS=10 cargo run -p test-channels-ftc --example basic_ftc --features hotpath
    #[test]
    fn test_data_endpoints() {
        assert_list_and_logs_endpoints(
            example("basic_ftc"),
            6772,
            "channels",
            &["basic_ftc.rs", "bounded-channel", "Actor 1"],
            |channels: &JsonChannelsList| channels.data.first().and_then(|channel| channel.id),
        );
    }
}
