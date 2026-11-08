//! Integration tests for the axum 0.8 server-side response time front-end.
//!
//! These run the `test-axum` `basic` example as a subprocess and assert on its
//! report and on the live `/server` metrics endpoint. The tower layer feeds
//! the `hp-server` worker, which buckets requests by matched route template.
#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use std::time::Duration;

    use hotpath::json::{JsonHttpLogsList, JsonServerList};

    use crate::common::assert_contains_all;
    use crate::common::example::{poll_endpoint, Example};

    fn example() -> Example {
        Example::new("test-axum", "basic")
    }

    #[test]
    fn test_server_table_axum() {
        let stdout = example().stdout();

        assert_contains_all(
            &stdout,
            &[
                "axum example completed",
                "server - HTTP server response time statistics per route.",
                // 3 user fetches + create + crash + unmatched 404 + fallback 200 = 7.
                "Total requests: 7",
                // Matched routes report the router's own template.
                "GET /users/{id}",
                "POST /users",
                "GET /crash",
                // Unmatched requests with an error status collapse per method.
                "GET <unmatched>",
                // Unmatched but fallback-served 2xx keeps the normalized raw path.
                "GET /pages/about",
            ],
        );
    }

    #[test]
    fn test_server_json_axum() {
        let server = example()
            .json()
            .report()
            .server
            .expect("No server section in report");

        assert_eq!(server.total_calls, 7);
        assert_eq!(
            server.total_calls,
            server.data.iter().map(|e| e.count).sum::<u64>()
        );

        let find = |route: &str| {
            server
                .data
                .iter()
                .find(|e| e.route == route)
                .unwrap_or_else(|| panic!("{route} bucket missing"))
        };

        // Two ids and a query string merge into the route template; the
        // matched-but-404 lookup lands in the same bucket as a 4xx.
        let users = find("GET /users/{id}");
        assert_eq!(users.count, 3);
        assert_eq!(users.status_4xx, 1);
        assert_eq!(users.status_5xx, 0);

        let created = find("POST /users");
        assert_eq!(created.count, 1);
        assert_eq!(created.status_4xx, 0);
        assert_eq!(created.status_5xx, 0);

        let crash = find("GET /crash");
        assert_eq!(crash.count, 1);
        assert_eq!(crash.status_5xx, 1);

        // Unmatched + error status collapses into the per-method bucket.
        let unmatched = find("GET <unmatched>");
        assert_eq!(unmatched.count, 1);
        assert_eq!(unmatched.status_4xx, 1);

        // Unmatched but served with 200 by the fallback keeps its own path.
        let about = find("GET /pages/about");
        assert_eq!(about.count, 1);
        assert_eq!(about.status_4xx, 0);
    }

    // HOTPATH_METRICS_PORT=6787 TEST_SLEEP_SECONDS=10 cargo run -p test-axum --example basic --features hotpath
    #[test]
    fn test_server_endpoints_axum() {
        let _running = example()
            .env("HOTPATH_METRICS_PORT", "6787")
            .env("TEST_SLEEP_SECONDS", "10")
            .spawn();

        let parse_server = |body: &str| -> JsonServerList {
            serde_json::from_str(body).expect("Failed to parse /server")
        };
        let body = poll_endpoint(
            "http://localhost:6787/server",
            40,
            Duration::from_millis(750),
            |body| parse_server(body).total_calls == 7,
        );
        let server = parse_server(&body);
        assert_eq!(server.total_calls, 7, "Never observed 7 served requests");

        let users = server
            .data
            .iter()
            .find(|e| e.route == "GET /users/{id}")
            .expect("users bucket missing");
        assert_eq!(users.count, 3);

        let logs_url = format!("http://localhost:6787/server/{}/logs", users.id.unwrap());
        let mut response = ureq::get(&logs_url).call().expect("GET server logs");
        let body = response
            .body_mut()
            .read_to_string()
            .expect("Failed to read logs body");
        let logs: JsonHttpLogsList = serde_json::from_str(&body).expect("Failed to parse logs");
        assert_eq!(logs.logs.len(), 3);
        // Newest first: the last GET /users/{id} was the 404 lookup.
        assert_eq!(logs.logs[0].status, "404");
        assert!(logs.logs.iter().skip(1).all(|l| l.status == "200"));

        let missing = ureq::get("http://localhost:6787/server/999999/logs").call();
        assert!(matches!(missing, Err(ureq::Error::StatusCode(404))));
    }
}
