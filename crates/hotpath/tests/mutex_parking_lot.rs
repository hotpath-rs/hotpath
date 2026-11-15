#[path = "common/support.rs"]
mod common;

#[cfg(test)]
pub mod tests {
    use crate::common::assert_contains_all;
    use crate::common::example::Example;

    fn example() -> Example {
        Example::new("test-mutex-parking-lot", "basic_mutex_parking_lot")
    }

    // cargo run -p test-mutex-parking-lot --example basic_mutex_parking_lot --features hotpath
    #[test]
    fn test_basic_output() {
        let stdout = example().stdout();

        assert_contains_all(
            &stdout,
            &[
                "parking_lot Mutex example completed!",
                "mutexes",
                "counter",
                "Locks",
                "Wait avg",
                "Acq avg",
            ],
        );
    }

    // cargo run -p test-mutex-parking-lot --example basic_mutex_parking_lot --features hotpath (json)
    #[test]
    fn test_json_output() {
        let stdout = example().json().stdout();

        assert_contains_all(
            &stdout,
            &[
                "\"mutexes\"",
                "\"name\":\"counter\"",
                "\"count\":6",
                "\"wait_avg\"",
                "\"acquire_avg\"",
                "\"wait_percentiles\"",
                "\"acquire_percentiles\"",
            ],
        );
    }
}
