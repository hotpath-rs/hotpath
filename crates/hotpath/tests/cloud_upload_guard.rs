#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
mod tests {
    //! The upload's verdict as a CI guard, against a mock hotpath.rs that also
    //! mints the OIDC token: the `HOTPATH_UPLOAD` mode decides the exit code and `HOTPATH_UPLOAD_RESPONSE_PATH` receives the response body.
    //!
    //! cargo test --features hotpath --test cloud_upload_guard -- --nocapture --test-threads=1

    use std::path::{Path, PathBuf};
    use std::process::Output;

    use hotpath::json::JsonReport;
    use mockito::{Matcher, Server, ServerGuard};

    use crate::common::example::Example;

    /// With a field this client does not know, which the response file keeps.
    const REGRESSED_BODY: &str = r#"{"id":"r1","new_field":{"z":1,"a":2},"repository":"hotpath-rs/hotpath","benchmark":"guard-test","baseline":"r0","comment":{"url":"https://github.com/c/1"},"verdict":{"judged":true,"regressed":true,"regressions":2,"improvements":0,"budgets_broken":1},"policy_path":"hotpath/policy.toml","policy_url":"https://github.com/hotpath-rs/hotpath/blob/3f1c000000000000000000000000000000000000/hotpath/policy.toml","dashboard_url":"https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/guard-test/reports/r1/diff"}"#;

    fn mock_server(status: usize, body: &str) -> (ServerGuard, mockito::Mock) {
        let mut server = Server::new();
        server
            .mock("GET", "/token")
            .match_query(Matcher::UrlEncoded("audience".into(), "hotpath.rs".into()))
            .with_status(200)
            .with_body(r#"{"value":"oidc-token"}"#)
            .create();
        let upload = server
            .mock("POST", "/api/v1/reports")
            .match_query(Matcher::UrlEncoded("benchmark".into(), "guard-test".into()))
            .match_header("authorization", "Bearer oidc-token")
            .with_status(status)
            .with_body(body)
            .create();
        (server, upload)
    }

    fn scratch_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hotpath_cloud_upload_guard_{test}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Runs from this crate, inside this repository, so the upload carries
    /// the repository's own `hotpath/policy.toml`.
    fn run_upload(server: &ServerGuard, dir: &Path, env: &[(&str, &str)]) -> Output {
        run_upload_in(None, server, dir, env)
    }

    // cargo run -p test-all-features --example basic_all_features --features hotpath,hotpath-cloud
    fn run_upload_in(
        cwd: Option<&Path>,
        server: &ServerGuard,
        dir: &Path,
        env: &[(&str, &str)],
    ) -> Output {
        let run = Example::new("test-all-features", "basic_all_features")
            .manifest_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml"))
            .features("hotpath,hotpath-cloud")
            .env("HOTPATH_REPORT", "functions-timing")
            .json()
            .env("HOTPATH_OUTPUT_PATH", dir.join("report.json"))
            .env("HOTPATH_BENCHMARK", "guard-test")
            .env("HOTPATH_UPLOAD", "1")
            .env("HOTPATH_API_URL", server.url())
            .env("HOTPATH_UPLOAD_RESPONSE_PATH", dir.join("response.json"))
            .env(
                "ACTIONS_ID_TOKEN_REQUEST_URL",
                format!("{}/token", server.url()),
            )
            .env("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "request-token")
            .env_remove("HOTPATH_POLICY_PATH")
            .env_remove("HOTPATH_SOURCE_ROOT")
            // Under Actions the child would annotate and summarize the test's own job.
            .env_remove("GITHUB_ACTIONS")
            .env_remove("GITHUB_STEP_SUMMARY");
        let run = match cwd {
            Some(cwd) => run.current_dir(cwd),
            None => run,
        };
        run.envs(env).output()
    }

    fn assert_local_report(dir: &Path) {
        let body =
            std::fs::read_to_string(dir.join("report.json")).expect("report file was not written");
        let report: JsonReport = serde_json::from_str(&body).expect("file is not a JSON report");
        assert!(report.functions_timing.is_some());
    }

    #[test]
    fn regressed_verdict_never_fails_the_job() {
        let dir = scratch_dir("regressed");
        let (server, upload) = mock_server(201, REGRESSED_BODY);

        // The server fails the pull request's check run; the job passes.
        let output = run_upload(&server, &dir, &[("HOTPATH_UPLOAD", "fail-on-error")]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        upload.assert();
        assert!(output.status.success(), "stderr:\n{stderr}");
        assert!(
            stderr.contains("; verdict: 2 regressions, 1 budget broken; diff: "),
            "stderr:\n{stderr}"
        );
        assert_local_report(&dir);
        let response =
            std::fs::read(dir.join("response.json")).expect("response file was not written");
        assert_eq!(response, REGRESSED_BODY.as_bytes());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_upload_fails_under_fail_on_error() {
        let dir = scratch_dir("failed");
        let stale = dir.join("response.json");
        std::fs::write(&stale, REGRESSED_BODY).unwrap();
        let (server, upload) = mock_server(500, r#"{"error":"database error","code":"internal"}"#);

        let output = run_upload(&server, &dir, &[("HOTPATH_UPLOAD", "fail-on-error")]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        upload.assert();
        assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
        assert!(
            stderr.contains("hotpath: upload failed: database error (HTTP 500"),
            "stderr:\n{stderr}"
        );
        assert_local_report(&dir);
        assert!(!stale.exists(), "a stale verdict was left behind");

        // An unknown mode still uploads, as `enabled`: a warning, and the job passes.
        let output = run_upload(&server, &dir, &[("HOTPATH_UPLOAD", "strict")]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "stderr:\n{stderr}");
        assert!(
            stderr.contains("hotpath: upload failed: database error (HTTP 500"),
            "stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("hotpath: unknown HOTPATH_UPLOAD \"strict\", uploading as \"enabled\""),
            "stderr:\n{stderr}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn report_without_a_policy_is_never_sent() {
        let dir = scratch_dir("no-policy");
        // A checkout of its own: this repository has a policy file.
        let checkout = dir.join("checkout");
        std::fs::create_dir_all(checkout.join(".git")).unwrap();
        let mut server = Server::new();
        let token = server.mock("GET", "/token").expect(0).create();
        let upload = server.mock("POST", "/api/v1/reports").expect(0).create();
        let refusal = "hotpath: upload failed: the report carries no policy file and hotpath.rs \
                       refuses reports without one. Add `hotpath/guard-test-policy.toml` or \
                       `hotpath/policy.toml` to the repository";

        let output = run_upload_in(Some(&checkout), &server, &dir, &[]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "stderr:\n{stderr}");
        assert!(stderr.contains(refusal), "stderr:\n{stderr}");
        assert_local_report(&dir);
        assert!(!dir.join("response.json").exists());

        // A file that is there but cannot be sent is refused the same way,
        // and a failed upload fails the job under fail-on-error.
        std::fs::create_dir_all(checkout.join("hotpath")).unwrap();
        std::fs::write(checkout.join("hotpath/policy.toml"), " \n").unwrap();
        let output = run_upload_in(
            Some(&checkout),
            &server,
            &dir,
            &[("HOTPATH_UPLOAD", "fail-on-error")],
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
        assert!(
            stderr.contains("hotpath: the policy file `hotpath/policy.toml` is blank."),
            "stderr:\n{stderr}"
        );
        assert!(stderr.contains(refusal), "stderr:\n{stderr}");
        assert_local_report(&dir);

        token.assert();
        upload.assert();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
