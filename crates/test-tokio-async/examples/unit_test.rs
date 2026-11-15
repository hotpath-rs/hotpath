//! Run with:
//!   cargo test -p test-tokio-async --example unit_test --features hotpath -- --nocapture --test-threads=1

use std::time::Duration;

/// Run with:
/// cargo test -p test-tokio-async --example unit_test --features hotpath -- --nocapture --test-threads=1
#[hotpath::measure]
fn sync_function(sleep: u64) {
    let vec1 = vec![1, 2, 3, 5, 6];
    std::hint::black_box(&vec1);
    drop(vec1);
    let vec2 = vec![1, 2, 3, 5, 6];
    std::hint::black_box(&vec2);
    std::thread::sleep(Duration::from_nanos(sleep));
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    for i in 0..100 {
        sync_function(i);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hotpath::json::{JsonChannelsList, JsonFunctionsList};

    #[test]
    fn test_sync_function_is_measured() {
        let temp_file = std::env::temp_dir().join("hotpath_unit_test.json");

        {
            let _hotpath = hotpath::HotpathGuardBuilder::new("test_sync_function")
                .format(hotpath::Format::Json)
                .output_path(&temp_file)
                .build();

            for _ in 0..10 {
                sync_function(100);
            }
        }

        let json_content = std::fs::read_to_string(&temp_file).expect("Failed to read output file");
        let wrapper: serde_json::Value =
            serde_json::from_str(&json_content).expect("Failed to parse JSON");
        let metrics: JsonFunctionsList =
            serde_json::from_value(wrapper["functions_timing"].clone())
                .expect("Failed to parse functions_timing section");

        let sync_fn_entry = metrics
            .data
            .iter()
            .find(|entry| entry.name == "unit_test::sync_function")
            .expect("sync_function should be in metrics");

        assert_eq!(
            sync_fn_entry.calls, 10,
            "Expected 10 calls to sync_function"
        );

        std::fs::remove_file(&temp_file).ok();
    }

    #[tokio::test]
    async fn test_channel_is_measured() {
        let temp_file = std::env::temp_dir().join("hotpath_channel_test.json");

        {
            let _channels_guard = hotpath::HotpathGuardBuilder::new("test_channel")
                .format(hotpath::Format::Json)
                .output_path(&temp_file)
                .sections(vec![hotpath::Section::Channels])
                .build();

            let (tx, mut rx) = hotpath::channel!(
                tokio::sync::mpsc::channel::<i32>(10),
                label = "test_channel"
            );

            for i in 0..5 {
                tx.send(i).await.expect("Failed to send");
            }

            drop(tx);

            while rx.recv().await.is_some() {}
        }

        let json_content = std::fs::read_to_string(&temp_file).expect("Failed to read output file");
        let wrapper: serde_json::Value =
            serde_json::from_str(&json_content).expect("Failed to parse JSON");
        let metrics: JsonChannelsList = serde_json::from_value(wrapper["channels"].clone())
            .expect("Failed to parse channels section");

        let channel_entry = metrics
            .data
            .iter()
            .find(|entry| entry.name == "test_channel")
            .expect("test_channel should be in metrics");

        assert_eq!(
            channel_entry.sent_count, 5,
            "Expected 5 messages sent on channel"
        );
        assert_eq!(
            channel_entry.received_count, 5,
            "Expected 5 messages received on channel"
        );

        std::fs::remove_file(&temp_file).ok();
    }
}
