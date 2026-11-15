#[path = "common/support.rs"]
mod common;

#[cfg(test)]
pub mod tests {
    use crate::common::assert_contains_all;
    use crate::common::example::Example;

    fn example() -> Example {
        Example::new("test-rw-lock-std", "basic_rw_lock_std")
    }

    // cargo run -p test-rw-lock-std --example basic_rw_lock_std --features hotpath
    #[test]
    fn test_basic_output() {
        let stdout = example().stdout();

        assert_contains_all(
            &stdout,
            &[
                "Std RwLock example completed!",
                "rw_locks",
                "counter",
                "Reads",
                "Writes",
                "Wait avg",
                "Acq avg",
            ],
        );
    }

    // cargo run -p test-rw-lock-std --example basic_rw_lock_std --features hotpath (json)
    #[test]
    fn test_json_output() {
        let stdout = example().json().stdout();

        assert_contains_all(
            &stdout,
            &[
                "\"rw_locks\"",
                "\"name\":\"counter\"",
                // The deprecated wrap constructor registers via Location::caller();
                // its auto label must keep the file:line form (the line must not be
                // eaten by the registration key's column strip).
                "\"name\":\"examples/basic_rw_lock_std.rs:",
                "\"read_count\":6",
                "\"write_count\":3",
                "\"read_wait_avg\"",
                "\"write_wait_avg\"",
                "\"read_acquire_avg\"",
                "\"write_acquire_avg\"",
                "\"read_wait_percentiles\"",
                "\"write_wait_percentiles\"",
                "\"read_acquire_percentiles\"",
                "\"write_acquire_percentiles\"",
            ],
        );
    }
}
