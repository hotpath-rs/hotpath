#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath-prometheus"))]
pub mod tests {
    use crate::common::example::Example;
    use crate::common::prometheus::{
        assert_histogram_family, assert_sample_lines_parse, label_value, parse_line, scrape_until,
    };

    const METRICS_PORT: &str = "6793";
    const PROMETHEUS_PORT: &str = "6794";

    // cargo run -p test-axum --example route_scope --features hotpath-prometheus
    #[test]
    fn test_sql_http_server_families() {
        let _running = Example::new("test-axum", "route_scope")
            .features("hotpath-prometheus")
            .env("HOTPATH_METRICS_PORT", METRICS_PORT)
            .env("HOTPATH_PROMETHEUS_PORT", PROMETHEUS_PORT)
            .env("TEST_SLEEP_SECONDS", "15")
            .spawn();

        // Wait for all 3 GET /profiles/{id} requests: sql/http attribution and request
        // completion arrive through different worker queues.
        let body = scrape_until(PROMETHEUS_PORT, 80, |body| {
            body.contains("hotpath_server_scoped_requests_total{route=\"GET /profiles/{id}\"} 3")
                && body.contains("hotpath_http_duration_seconds_bucket")
        });

        for family in [
            "hotpath_sql_queries_total",
            "hotpath_sql_duration_seconds",
            "hotpath_http_requests_total",
            "hotpath_http_errors_total",
            "hotpath_http_duration_seconds",
            "hotpath_server_requests_total",
            "hotpath_server_responses_total",
            "hotpath_server_duration_seconds",
            "hotpath_server_scoped_requests_total",
            "hotpath_server_sql_calls_total",
            "hotpath_server_http_calls_total",
        ] {
            assert!(
                body.contains(&format!("# TYPE {family} ")),
                "missing family {family}, body:\n{body}"
            );
        }

        assert_sample_lines_parse(&body);

        for family in [
            "hotpath_sql_duration_seconds",
            "hotpath_http_duration_seconds",
            "hotpath_server_duration_seconds",
        ] {
            assert_histogram_family(&body, family);
        }

        // SQL series are identified by normalized query text plus the
        // route/source attribution.
        let duration_count = body
            .lines()
            .find(|l| {
                l.starts_with("hotpath_sql_duration_seconds_count{")
                    && l.contains("query=\"SELECT COUNT(*) FROM users\"")
            })
            .expect("duration series for COUNT query missing");
        let (_, labels, _) = parse_line(duration_count).unwrap();
        assert_eq!(label_value(labels, "route"), "GET /profiles/{id}");
        assert_eq!(label_value(labels, "source"), "route_scope::count_users");

        // The unmatched /missing request produced a 4xx response.
        let has_4xx = body.lines().any(|l| {
            parse_line(l).is_some_and(|(name, labels, value)| {
                name == "hotpath_server_responses_total"
                    && labels.contains("class=\"4xx\"")
                    && value.parse::<f64>().unwrap_or(0.0) >= 1.0
            })
        });
        assert!(has_4xx, "no 4xx responses counted, body:\n{body}");

        // Route-scoped attribution: GET /profiles/{id} issues 2 SQL
        // queries and 1 outbound HTTP request per request.
        let scoped_value = |name: &str| {
            body.lines()
                .find_map(|l| {
                    parse_line(l).filter(|(n, labels, _)| {
                        *n == name && label_value(labels, "route") == "GET /profiles/{id}"
                    })
                })
                .map(|(_, _, value)| value.parse::<f64>().unwrap())
                .unwrap_or_else(|| panic!("{name} series for GET /profiles/{{id}} missing"))
        };
        assert_eq!(scoped_value("hotpath_server_scoped_requests_total"), 3.0);
        assert_eq!(scoped_value("hotpath_server_sql_calls_total"), 6.0);
        assert_eq!(scoped_value("hotpath_server_http_calls_total"), 3.0);

        // Timing-mode functions are split by route too: load_user runs
        // under both routes (2 + 3 calls), count_users under one.
        let function_route_value = |name: &str, function: &str, route: &str| {
            body.lines()
                .find_map(|l| {
                    parse_line(l).filter(|(n, labels, _)| {
                        *n == name
                            && label_value(labels, "function") == function
                            && label_value(labels, "route") == route
                    })
                })
                .map(|(_, _, value)| value.parse::<f64>().unwrap())
                .unwrap_or_else(|| panic!("{name} series for {function} / {route} missing"))
        };
        let calls = "hotpath_function_route_calls_total";
        assert_eq!(
            function_route_value(calls, "route_scope::load_user", "GET /users/{id}"),
            5.0
        );
        assert_eq!(
            function_route_value(calls, "route_scope::load_user", "GET /profiles/{id}"),
            3.0
        );
        assert_eq!(
            function_route_value(calls, "route_scope::count_users", "GET /profiles/{id}"),
            3.0
        );
        assert!(
            function_route_value(
                "hotpath_function_route_duration_seconds_total",
                "route_scope::load_user",
                "GET /profiles/{id}"
            ) > 0.0
        );
        assert!(
            !body.contains("hotpath_function_route_alloc_bytes_total"),
            "alloc route families exported without hotpath-alloc"
        );
    }

    // cargo run -p test-axum --example route_alloc --features hotpath-alloc,hotpath-prometheus
    #[test]
    fn test_server_alloc_families() {
        let _running = Example::new("test-axum", "route_alloc")
            .features("hotpath-alloc,hotpath-prometheus")
            .env("HOTPATH_METRICS_PORT", METRICS_PORT)
            .env("HOTPATH_PROMETHEUS_PORT", PROMETHEUS_PORT)
            .env("TEST_SLEEP_SECONDS", "15")
            .spawn();

        // The example serves exactly 3 GET /big requests.
        let body = scrape_until(PROMETHEUS_PORT, 80, |body| {
            body.contains("hotpath_server_scoped_requests_total{route=\"GET /big\"} 3")
        });

        for family in [
            "hotpath_server_alloc_bytes_total",
            "hotpath_server_alloc_count_total",
            "hotpath_server_alloc_bytes",
        ] {
            assert!(
                body.contains(&format!("# TYPE {family} ")),
                "missing family {family}, body:\n{body}"
            );
        }
        assert_histogram_family(&body, "hotpath_server_alloc_bytes");

        let route_value = |name: &str, route: &str| {
            body.lines()
                .find_map(|l| {
                    parse_line(l).filter(|(n, labels, _)| {
                        *n == name && label_value(labels, "route") == route
                    })
                })
                .map(|(_, _, value)| value.parse::<f64>().unwrap())
                .unwrap_or_else(|| panic!("{name} series for {route} missing"))
        };
        // Three 1 MiB bodies, and at least one allocation each.
        let big_bytes = route_value("hotpath_server_alloc_bytes_total", "GET /big");
        assert!(big_bytes >= 3.0 * 1024.0 * 1024.0, "{big_bytes}");
        assert!(route_value("hotpath_server_alloc_count_total", "GET /big") >= 3.0);
        assert_eq!(
            route_value("hotpath_server_alloc_bytes_count", "GET /big"),
            3.0
        );
        assert_eq!(
            route_value("hotpath_server_alloc_bytes_sum", "GET /big"),
            big_bytes
        );
        assert!(route_value("hotpath_server_alloc_bytes_total", "GET /small") < 64.0 * 1024.0);

        // Unmatched requests carry no scope: counters exist at zero, but
        // there is no per-request distribution to export.
        assert_eq!(
            route_value("hotpath_server_alloc_bytes_total", "GET <unmatched>"),
            0.0
        );
        assert!(
            !body.contains("hotpath_server_alloc_bytes_count{route=\"GET <unmatched>\"}"),
            "unscoped route exported a histogram, body:\n{body}"
        );

        // Measured functions are split by the route they ran under.
        let function_route_value = |name: &str| {
            body.lines()
                .find_map(|l| {
                    parse_line(l).filter(|(n, labels, _)| {
                        *n == name
                            && label_value(labels, "function") == "route_alloc::build_body"
                            && label_value(labels, "route") == "GET /big"
                    })
                })
                .map(|(_, _, value)| value.parse::<f64>().unwrap())
                .unwrap_or_else(|| panic!("{name} series for build_body / GET /big missing"))
        };
        assert_eq!(
            function_route_value("hotpath_function_route_calls_total"),
            3.0
        );
        assert_eq!(
            function_route_value("hotpath_function_route_timed_calls_total"),
            3.0
        );
        assert!(function_route_value("hotpath_function_route_duration_seconds_total") > 0.0);
        assert!(
            function_route_value("hotpath_function_route_alloc_bytes_total")
                >= 3.0 * 1024.0 * 1024.0
        );
        assert_eq!(
            function_route_value("hotpath_function_route_alloc_count_total"),
            3.0
        );
    }
}
