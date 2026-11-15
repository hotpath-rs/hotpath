#[path = "common/support.rs"]
mod common;

#[cfg(test)]
pub mod tests {
    use crate::common::assert_contains_all;
    use crate::common::example::Example;

    fn example(name: &str) -> Example {
        Example::new("test-mutex-std", name)
    }

    // cargo run -p test-mutex-std --example basic_mutex_std --features hotpath
    #[test]
    fn test_basic_output() {
        let stdout = example("basic_mutex_std").stdout();

        assert_contains_all(
            &stdout,
            &[
                "Std Mutex example completed!",
                "mutexes",
                "counter",
                "Locks",
                "Wait avg",
                "Acq avg",
            ],
        );
    }

    // cargo run -p test-mutex-std --example basic_mutex_std --features hotpath (json)
    #[test]
    fn test_json_output() {
        let stdout = example("basic_mutex_std").json().stdout();

        assert_contains_all(
            &stdout,
            &[
                "\"mutexes\"",
                "\"name\":\"counter\"",
                // The deprecated wrap constructor registers via Location::caller();
                // its auto label must keep the file:line form (the line must not be
                // eaten by the registration key's column strip).
                "\"name\":\"examples/basic_mutex_std.rs:",
                "\"count\":6",
                "\"wait_avg\"",
                "\"acquire_avg\"",
                "\"wait_percentiles\"",
                "\"acquire_percentiles\"",
            ],
        );
    }

    // Locks recorded on a thread whose event queue drains ahead of the queue
    // carrying `Created` must still be counted (placeholder backfill).
    // cargo run -p test-mutex-std --example created_ordering --features hotpath (json)
    #[test]
    fn test_created_ordering_output() {
        let stdout = example("created_ordering").json().stdout();

        assert_contains_all(&stdout, &["\"name\":\"target\"", "\"count\":100"]);
    }
}
