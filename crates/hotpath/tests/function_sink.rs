#[path = "common/support.rs"]
mod common;

#[cfg(test)]
pub mod tests {
    use crate::common::example::Example;

    fn run(features: &str) -> String {
        Example::new("test-tokio-async", "function_sink")
            .features(features)
            .env("HOTPATH_METRICS_SERVER_OFF", "true")
            .stdout()
    }

    // cargo run -p test-tokio-async --example function_sink --features hotpath
    #[test]
    fn test_function_sink_timing() {
        assert!(run("hotpath").contains("function_sink ok: 132 events"));
    }

    // cargo run -p test-tokio-async --example function_sink --features hotpath-alloc
    #[test]
    fn test_function_sink_alloc() {
        assert!(run("hotpath-alloc").contains("function_sink ok: 132 events"));
    }
}
