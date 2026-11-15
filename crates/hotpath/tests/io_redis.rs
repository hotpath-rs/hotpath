//! Integration test for `io!` instrumentation over a real Redis TCP
//! connection. Runs the `test-io` `basic_redis_io` example as a subprocess and
//! asserts on its report. Requires the Redis container from the repo-root
//! compose file (`docker compose up -d redis`, host port 6390). Skips locally
//! when nothing listens there; on CI the redis service is mandatory, so a
//! missing server fails instead.
#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use crate::common::example::Example;
    use crate::common::service::REDIS;

    #[test]
    fn test_redis_json_output() {
        if REDIS.skip_if_unavailable("test_redis_json_output") {
            return;
        }

        let report = Example::new("test-io", "basic_redis_io").json().report();
        let io = report.io.expect("No io section in report");

        let redis = io
            .data
            .iter()
            .find(|e| e.name == "redis")
            .expect("No 'redis' entry in io section");

        assert!(redis.type_name.contains("TcpStream"));
        // SET + GET + PING + DEL requests, one write each.
        assert_eq!(redis.write.count, 4);
        assert_eq!(redis.write.bytes, (49 + 31 + 14 + 31) as u64);
        assert_eq!(redis.write.errors, 0);
        // Four responses (+OK, $11\r\nhotpath-val\r\n, +PONG, :1); each needs
        // at least one read, but the kernel may split a response.
        assert!(redis.read.count >= 4);
        assert_eq!(redis.read.bytes, (5 + 18 + 7 + 4) as u64);
        assert_eq!(redis.read.errors, 0);
        assert!(redis.read.total_ns > 0, "Reads should be timed");
    }
}
