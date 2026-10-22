#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath-prometheus"))]
pub mod tests {
    use std::{thread::sleep, time::Duration};

    use crate::common::example::Example;
    use crate::common::prometheus::{
        assert_histogram_family, assert_sample_lines_parse, label_value,
    };

    const METRICS_PORT: &str = "6791";
    const PROMETHEUS_PORT: &str = "6792";
    const TOKEN: &str = "prom-secret";
    const FAST_BUCKETS: &str = "0.000001,0.00002,0.5";

    fn get(path: &str, auth: Option<&str>) -> Result<(u16, String, String), ureq::Error> {
        let url = format!("http://localhost:{}{}", PROMETHEUS_PORT, path);
        let mut request = ureq::get(&url).config().http_status_as_error(false).build();
        if let Some(token) = auth {
            request = request.header("Authorization", token);
        }
        let mut response = request.call()?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.body_mut().read_to_string()?;
        Ok((status, content_type, body))
    }

    fn assert_custom_fast_buckets(body: &str) {
        let les: Vec<String> = body
            .lines()
            .filter(|l| l.starts_with("hotpath_function_duration_seconds_bucket{"))
            .filter(|l| l.contains("function=\"basic::sync_function\""))
            .map(|l| label_value(l, "le"))
            .collect();
        assert_eq!(
            les,
            ["0.000001", "0.00002", "0.5", "+Inf"],
            "HOTPATH_PROMETHEUS_FAST_BUCKETS not applied, body:\n{body}"
        );
    }

    // cargo run -p test-tokio-async --example basic --features hotpath-prometheus
    #[test]
    fn test_prometheus_endpoint() {
        let _running = Example::new("test-tokio-async", "basic")
            .features("hotpath-prometheus")
            .env("HOTPATH_METRICS_PORT", METRICS_PORT)
            .env("HOTPATH_PROMETHEUS_PORT", PROMETHEUS_PORT)
            .env("HOTPATH_PROMETHEUS_AUTH_TOKEN", TOKEN)
            .env("HOTPATH_PROMETHEUS_FAST_BUCKETS", FAST_BUCKETS)
            .env("TEST_SLEEP_SECONDS", "15")
            .spawn();

        let mut ready = false;
        for _attempt in 0..60 {
            sleep(Duration::from_millis(750));
            if let Ok((200, _, body)) = get("/metrics", Some(TOKEN)) {
                if body.contains("hotpath_function_duration_seconds_bucket") {
                    ready = true;
                    break;
                }
            }
        }
        assert!(
            ready,
            "Prometheus server did not serve metrics on port {PROMETHEUS_PORT}"
        );

        let (status, _, _) = get("/metrics", None).expect("request without token");
        assert_eq!(status, 401, "missing token");

        let (status, _, _) = get("/metrics", Some("wrong-token")).expect("wrong token");
        assert_eq!(status, 401, "wrong token");

        let (status, _, _) =
            get("/metrics", Some(&format!("Bearer {}", TOKEN))).expect("bearer token");
        assert_eq!(status, 200, "Bearer-prefixed token");

        let (status, _, _) = get("/unknown", Some(TOKEN)).expect("unknown path");
        assert_eq!(status, 404, "unknown path");

        let (status, content_type, body) = get("/metrics", Some(TOKEN)).expect("scrape");
        assert_eq!(status, 200);
        assert_eq!(content_type, "text/plain; version=0.0.4; charset=utf-8");

        for family in [
            "hotpath_build_info",
            "hotpath_uptime_seconds",
            "hotpath_function_calls_total",
            "hotpath_function_duration_seconds",
        ] {
            assert!(
                body.contains(&format!("# TYPE {family} ")),
                "missing family {family}, body:\n{body}"
            );
        }
        assert!(
            body.contains("hotpath_build_info{hotpath_version=\"")
                && body.contains("rustc_version=\"")
                && body.contains("profile=\"debug\"")
                && body.contains(&format!(
                    "os=\"{}-{}\"",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )),
            "build info labels missing, body:\n{body}"
        );
        assert!(
            body.contains("function=\"basic::sync_function\""),
            "instrumented function missing, body:\n{body}"
        );

        assert_sample_lines_parse(&body);

        assert_histogram_family(&body, "hotpath_function_duration_seconds");
        assert_custom_fast_buckets(&body);
    }
}
