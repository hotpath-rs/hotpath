#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath-prometheus"))]
pub mod tests {
    use prost::Message;
    use std::{thread::sleep, time::Duration};

    use crate::common::example::Example;

    const METRICS_PORT: &str = "6796";
    const PROMETHEUS_PORT: &str = "6797";
    const PROTOBUF_ACCEPT: &str =
        "application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited";

    // Independent mirror of io.prometheus.client, declared from the upstream
    // field table rather than reusing the exporter's structs, so decoding
    // cross-checks the emitted tags. The enum is read as a raw varint.
    #[derive(Clone, PartialEq, Message)]
    struct MetricFamily {
        #[prost(string, optional, tag = "1")]
        name: Option<String>,
        #[prost(int32, optional, tag = "3")]
        r#type: Option<i32>,
        #[prost(message, repeated, tag = "4")]
        metric: Vec<Metric>,
    }

    #[derive(Clone, PartialEq, Message)]
    struct Metric {
        #[prost(message, repeated, tag = "1")]
        label: Vec<LabelPair>,
        #[prost(message, optional, tag = "3")]
        counter: Option<Counter>,
        #[prost(message, optional, tag = "7")]
        histogram: Option<Histogram>,
    }

    #[derive(Clone, PartialEq, Message)]
    struct LabelPair {
        #[prost(string, optional, tag = "1")]
        name: Option<String>,
        #[prost(string, optional, tag = "2")]
        value: Option<String>,
    }

    #[derive(Clone, PartialEq, Message)]
    struct Counter {
        #[prost(double, optional, tag = "1")]
        value: Option<f64>,
    }

    #[derive(Clone, PartialEq, Message)]
    struct Histogram {
        #[prost(uint64, optional, tag = "1")]
        sample_count: Option<u64>,
        #[prost(double, optional, tag = "2")]
        sample_sum: Option<f64>,
        #[prost(message, repeated, tag = "3")]
        bucket: Vec<Bucket>,
        #[prost(sint32, optional, tag = "5")]
        schema: Option<i32>,
        #[prost(message, repeated, tag = "12")]
        positive_span: Vec<BucketSpan>,
        #[prost(sint64, repeated, packed = "false", tag = "13")]
        positive_delta: Vec<i64>,
    }

    #[derive(Clone, PartialEq, Message)]
    struct Bucket {
        #[prost(uint64, optional, tag = "1")]
        cumulative_count: Option<u64>,
        #[prost(double, optional, tag = "2")]
        upper_bound: Option<f64>,
    }

    #[derive(Clone, PartialEq, Message)]
    struct BucketSpan {
        #[prost(sint32, optional, tag = "1")]
        offset: Option<i32>,
        #[prost(uint32, optional, tag = "2")]
        length: Option<u32>,
    }

    fn get(accept: Option<&str>) -> Result<(u16, String, Vec<u8>), ureq::Error> {
        let url = format!("http://localhost:{}/metrics", PROMETHEUS_PORT);
        let mut request = ureq::get(&url).config().http_status_as_error(false).build();
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }
        let mut response = request.call()?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response.body_mut().read_to_vec()?;
        Ok((status, content_type, body))
    }

    fn decode_families(mut buf: &[u8]) -> Vec<MetricFamily> {
        let mut families = Vec::new();
        while !buf.is_empty() {
            families.push(
                MetricFamily::decode_length_delimited(&mut buf).expect("decode MetricFamily"),
            );
        }
        families
    }

    fn label_value(metric: &Metric, name: &str) -> String {
        metric
            .label
            .iter()
            .find(|l| l.name.as_deref() == Some(name))
            .and_then(|l| l.value.clone())
            .unwrap_or_default()
    }

    // cargo run -p test-tokio-async --example basic --features hotpath-prometheus
    #[test]
    fn test_native_histograms_protobuf() {
        let _running = Example::new("test-tokio-async", "basic")
            .features("hotpath-prometheus")
            .env("HOTPATH_METRICS_PORT", METRICS_PORT)
            .env("HOTPATH_PROMETHEUS_PORT", PROMETHEUS_PORT)
            .env("TEST_SLEEP_SECONDS", "15")
            .spawn();

        // Wait until the workload finished (100 calls) so both bodies below
        // snapshot identical, static stats.
        let mut ready = false;
        for _attempt in 0..60 {
            sleep(Duration::from_millis(750));
            if let Ok((200, _, body)) = get(None) {
                let text = String::from_utf8_lossy(&body).to_string();
                if text
                    .contains("hotpath_function_calls_total{function=\"basic::sync_function\"} 100")
                {
                    ready = true;
                    break;
                }
            }
        }
        assert!(
            ready,
            "Prometheus server did not serve metrics on port {PROMETHEUS_PORT}"
        );

        let (status, content_type, text_body) = get(None).expect("text scrape");
        assert_eq!(status, 200);
        assert_eq!(content_type, "text/plain; version=0.0.4; charset=utf-8");
        let text = String::from_utf8_lossy(&text_body).to_string();
        assert!(text.contains("hotpath_function_duration_seconds_bucket"));

        let (status, content_type, proto_body) =
            get(Some(PROTOBUF_ACCEPT)).expect("protobuf scrape");
        assert_eq!(status, 200);
        assert_eq!(
            content_type,
            "application/vnd.google.protobuf; proto=io.prometheus.client.MetricFamily; encoding=delimited"
        );

        let families = decode_families(&proto_body);
        let names: Vec<_> = families.iter().filter_map(|f| f.name.clone()).collect();
        for expected in [
            "hotpath_build_info",
            "hotpath_uptime_seconds",
            "hotpath_function_calls_total",
            "hotpath_function_duration_seconds",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }

        let durations = families
            .iter()
            .find(|f| f.name.as_deref() == Some("hotpath_function_duration_seconds"))
            .unwrap();
        assert_eq!(durations.r#type, Some(4), "HISTOGRAM enum value");
        assert!(!durations.metric.is_empty());

        for metric in &durations.metric {
            let function = label_value(metric, "function");
            let hist = metric.histogram.as_ref().expect("histogram set");
            let sample_count = hist.sample_count.unwrap();
            assert_eq!(hist.schema, Some(3), "{function}: schema");

            // Native part: spans cover exactly the deltas, and the
            // reconstructed bucket counts sum to sample_count.
            let span_len: u32 = hist.positive_span.iter().map(|s| s.length.unwrap()).sum();
            assert_eq!(span_len as usize, hist.positive_delta.len(), "{function}");
            let mut cumulative = 0i64;
            let mut total = 0u64;
            for delta in &hist.positive_delta {
                cumulative += delta;
                assert!(cumulative >= 0, "{function}: negative bucket count");
                total += cumulative as u64;
            }
            assert_eq!(total, sample_count, "{function}: native counts vs count");

            // Classic part: full ladder, cumulative, consistent with the
            // text body scraped from the same static stats.
            assert_eq!(hist.bucket.len(), 16, "{function}: ladder size");
            let counts: Vec<u64> = hist
                .bucket
                .iter()
                .map(|b| b.cumulative_count.unwrap())
                .collect();
            assert!(counts.windows(2).all(|w| w[0] <= w[1]), "{function}");
            assert!(*counts.last().unwrap() <= sample_count, "{function}");
            let text_count_line = format!(
                "hotpath_function_duration_seconds_count{{function=\"{function}\"}} {sample_count}"
            );
            assert!(
                text.contains(&text_count_line),
                "{function}: text/proto count mismatch, wanted: {text_count_line}"
            );
        }

        let calls = families
            .iter()
            .find(|f| f.name.as_deref() == Some("hotpath_function_calls_total"))
            .unwrap();
        let sync_calls = calls
            .metric
            .iter()
            .find(|m| label_value(m, "function") == "basic::sync_function")
            .unwrap();
        assert_eq!(sync_calls.counter.as_ref().unwrap().value, Some(100.0));
    }
}
