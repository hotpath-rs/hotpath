#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
mod tests {
    use hotpath::json::JsonReport;

    use crate::common::example::Example;

    /// The fork case: a job that measures untrusted code gets no OIDC token, so
    /// it writes the JSON report for a trusted job to upload. Nothing may be
    /// posted, and the file must carry what an upload would have sent.
    // cargo run -p test-all-features --example basic_all_features --features hotpath-cloud
    #[test]
    fn json_file_is_an_upload_payload() {
        let path = std::env::temp_dir().join("hotpath_cloud_json_file_test.json");
        let _ = std::fs::remove_file(&path);

        let output = Example::new("test-all-features", "basic_all_features")
            .features("hotpath-cloud")
            .env("HOTPATH_REPORT", "functions-timing")
            .json()
            .env("HOTPATH_OUTPUT_PATH", &path)
            .env("HOTPATH_BENCHMARK", "fork-test")
            .env("HOTPATH_LIMIT", "1")
            .env_remove("ACTIONS_ID_TOKEN_REQUEST_URL")
            .env_remove("ACTIONS_ID_TOKEN_REQUEST_TOKEN")
            .env_remove("HOTPATH_UPLOAD")
            .output();
        assert!(
            output.status.success(),
            "Process did not exit successfully.\n\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("upload"),
            "nothing should attempt an upload:\n{stderr}"
        );

        let body = std::fs::read_to_string(&path).expect("report file was not written");
        let report: JsonReport = serde_json::from_str(&body).expect("file is not a JSON report");
        assert_eq!(report.meta.benchmark.as_deref(), Some("fork-test"));
        assert!(
            report.meta.git.is_some(),
            "no git provenance in the payload"
        );

        let functions = report
            .functions_timing
            .expect("No functions_timing section in report");
        assert!(
            functions.total_count > 1,
            "example must measure several functions"
        );
        assert_eq!(
            functions.included_count, functions.total_count,
            "payload must ignore HOTPATH_LIMIT"
        );

        let _ = std::fs::remove_file(&path);
    }
}
