#[cfg(all(test, feature = "cloud"))]
mod tests {
    //! `hotpath cloud auth|repos|benchmarks|report|diff|validate-policy|init`
    //! against a mock hotpath.rs: the bearer request each sends, the 2xx body
    //! it passes through as received (unknown fields and the server's key
    //! order included), the error JSON on stderr (server bodies verbatim, client
    //! failures as `{"error": ...}`) with exit 1, `diff`'s exit 0 or 1 read
    //! from its body, argument validation before any request (and before the
    //! token is read), clap usage errors with exit 2, and that the token never
    //! reaches stdout or stderr.
    //!
    //! cargo test -p hotpath --features cloud --test cloud_cli

    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    use hotpath::json::cloud_api::{ApiError, ApiErrorCode, POLICY_MAX_BYTES};
    use mockito::{Matcher, Server, ServerGuard};
    use serde_json::json;

    const TOKEN: &str = "hpat_5f3c9a1b2d4e6f7a8b9c0d1e2f3a4b5c";
    const AUTH_BODY: &str =
        r#"{"login":"pawurb","token":{"name":"laptop","expires_at":"2027-01-01T00:00:00Z"}}"#;
    const REPOS_BODY: &str = r#"{"repositories":[{"full_name":"hotpath-rs/hotpath","private":false,"visibility_public":true,"benchmarks":[{"name":"ci","reports":412,"latest_report_at":"2026-09-25T18:03:11Z"},{"name":"empty","reports":0,"latest_report_at":null}]},{"full_name":"pawurb/private-thing","private":true,"visibility_public":false,"benchmarks":[]}]}"#;
    const BENCHMARKS_BODY: &str = r#"{"repository":"hotpath-rs/hotpath","benchmarks":[{"name":"ci","reports":412,"latest_report_at":"2026-09-25T18:03:11Z"}]}"#;
    const BENCHMARKS_PATH: &str = "/api/v1/repos/hotpath-rs/hotpath/benchmarks";
    const REPORTS_PATH: &str = "/api/v1/repos/hotpath-rs/hotpath/benchmarks/ci/reports";
    const REPORT_ID: &str = "0199a3c2-7d2e-7b41-9c3a-1f2e3d4c5b6a";
    const SHA: &str = "9ab2000000000000000000000000000000000000";
    const SUMMARY_BODY: &str = r#"{"id":"0199a3c2-7d2e-7b41-9c3a-1f2e3d4c5b6a","repository":"hotpath-rs/hotpath","benchmark":"ci","event":"pull_request","commit_sha":"3f1c000000000000000000000000000000000000","head_sha":"9ab2000000000000000000000000000000000000","base_sha":"77de000000000000000000000000000000000000","git_ref":null,"base_ref":"main","head_ref":"channel-delay","pr_number":105,"run_id":"18237461234","workflow":"CI","actor":"pawurb","ci_provider":"github-actions","hotpath_version":"0.26.1","user_metadata":{"profile":"release"},"baseline_id":"0199a3b0-0000-7000-8000-000000000000","comment_url":"https://github.com/hotpath-rs/hotpath/pull/105#issuecomment-1","regressed":false,"policy_path":"hotpath/ci-policy.toml","policy_url":"https://github.com/hotpath-rs/hotpath/blob/3f1c000000000000000000000000000000000000/hotpath/ci-policy.toml","size_bytes":81234,"created_at":"2026-09-25T18:03:11Z","dashboard_url":"https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/ci/reports/0199a3c2-7d2e-7b41-9c3a-1f2e3d4c5b6a"}"#;

    /// `SUMMARY_BODY` plus a payload in the writer's (unsorted) key order.
    fn report_body() -> String {
        format!(
            "{},\"payload\":{{\"version\":\"0.26.1\",\"meta\":{{}}}}}}",
            &SUMMARY_BODY[..SUMMARY_BODY.len() - 1]
        )
    }

    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("not JSON: {e}\n{text}"))
    }

    /// A 200 mock of a report route with this exact query.
    fn mock_report(server: &mut ServerGuard, path: &str, query: &str, body: &str) -> mockito::Mock {
        server
            .mock("GET", path)
            .match_query(Matcher::Exact(query.into()))
            .match_header("authorization", format!("Bearer {TOKEN}").as_str())
            .with_status(200)
            .with_header("content-type", "application/json; charset=utf-8")
            .with_body(body)
            .create()
    }

    fn report_args<'a>(selector: &[&'a str]) -> Vec<&'a str> {
        let mut args = vec![
            "report",
            "--repo",
            "hotpath-rs/hotpath",
            "--benchmark",
            "ci",
        ];
        args.extend_from_slice(selector);
        args
    }

    fn hotpath(server: &ServerGuard, token: Option<&str>, args: &[&str]) -> Output {
        hotpath_with_stdin(server, token, args, b"")
    }

    /// `hotpath cloud <args>` with `stdin` piped in.
    fn hotpath_with_stdin(
        server: &ServerGuard,
        token: Option<&str>,
        args: &[&str],
        stdin: &[u8],
    ) -> Output {
        hotpath_in(server, token, args, stdin, None, None)
    }

    /// `hotpath cloud <args>` run from `cwd`, with `HOTPATH_POLICY_PATH` set
    /// to `policy_path`.
    fn hotpath_in(
        server: &ServerGuard,
        token: Option<&str>,
        args: &[&str],
        stdin: &[u8],
        cwd: Option<&Path>,
        policy_path: Option<&str>,
    ) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hotpath"));
        cmd.arg("cloud")
            .args(args)
            .env("HOTPATH_API_URL", format!("{}/", server.url()))
            .env_remove("HOTPATH_API_TOKEN")
            .env_remove("HOTPATH_POLICY_PATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(token) = token {
            cmd.env("HOTPATH_API_TOKEN", token);
        }
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        if let Some(policy_path) = policy_path {
            cmd.env("HOTPATH_POLICY_PATH", policy_path);
        }
        let mut child = cmd.spawn().expect("failed to run the hotpath binary");
        // The binary may exit without reading stdin, so a broken pipe is fine.
        let _ = child.stdin.take().unwrap().write_all(stdin);
        let output = child
            .wait_with_output()
            .expect("failed to run the hotpath binary");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stdout.contains(TOKEN) && !stderr.contains(TOKEN),
            "the token leaked into the output:\nstdout: {stdout}\nstderr: {stderr}"
        );
        output
    }

    fn stdout(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    /// The `error` of a client-built failure on stderr, which is always one
    /// JSON object with that single key.
    fn client_error(output: &Output) -> String {
        let stderr = stderr(output);
        let value: serde_json::Value = serde_json::from_str(&stderr).unwrap_or_else(|e| {
            panic!("stderr is not JSON: {e}\n{stderr}");
        });
        let object = value.as_object().expect("stderr is not a JSON object");
        assert_eq!(object.len(), 1, "{stderr}");
        object["error"]
            .as_str()
            .expect("error is not a string")
            .to_string()
    }

    fn error_body(code: ApiErrorCode, error: &str) -> String {
        serde_json::to_string(&ApiError {
            error: error.into(),
            code,
        })
        .unwrap()
    }

    fn mock_get(server: &mut ServerGuard, path: &str, body: &str) -> mockito::Mock {
        server
            .mock("GET", path)
            .match_header("authorization", format!("Bearer {TOKEN}").as_str())
            .match_header("user-agent", Matcher::Regex("^hotpath-cli/[0-9]".into()))
            .with_status(200)
            .with_header("content-type", "application/json; charset=utf-8")
            .with_body(body)
            .create()
    }

    fn mock_auth(server: &mut ServerGuard) -> mockito::Mock {
        mock_get(server, "/api/v1/auth", AUTH_BODY)
    }

    #[test]
    fn auth_prints_the_status_body_compact_by_default() {
        let mut server = Server::new();
        let mock = mock_auth(&mut server);

        let output = hotpath(&server, Some(TOKEN), &["auth"]);
        mock.assert();
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(stdout(&output), format!("{AUTH_BODY}\n"));
        assert_eq!(stderr(&output), "");

        assert_eq!(
            json(&stdout(&output)),
            json!({
                "login": "pawurb",
                "token": {"name": "laptop", "expires_at": "2027-01-01T00:00:00Z"}
            })
        );
    }

    #[test]
    fn auth_passes_unknown_fields_and_the_server_key_order_through() {
        // Keys out of alphabetical order, a field no client type knows and
        // server spacing: compact output is the body as received.
        let body = r#"{"token":{"name":"laptop","expires_at":"2027-01-01T00:00:00Z","scopes":["read"]}, "login":"pawurb","new_field":1}"#;
        let mut server = Server::new();
        let mock = mock_get(&mut server, "/api/v1/auth", body).expect(2);

        let output = hotpath(&server, Some(TOKEN), &["auth"]);
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(stdout(&output), format!("{body}\n"));

        let output = hotpath(&server, Some(TOKEN), &["auth", "--pretty"]);
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(
            stdout(&output),
            r#"{
  "token": {
    "name": "laptop",
    "expires_at": "2027-01-01T00:00:00Z",
    "scopes": [
      "read"
    ]
  },
  "login": "pawurb",
  "new_field": 1
}
"#
        );
        mock.assert();
    }

    #[test]
    fn auth_pretty_and_output_file() {
        let mut server = Server::new();
        let mock = mock_auth(&mut server).expect(2);

        let output = hotpath(&server, Some(TOKEN), &["auth", "--pretty"]);
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(
            stdout(&output),
            r#"{
  "login": "pawurb",
  "token": {
    "name": "laptop",
    "expires_at": "2027-01-01T00:00:00Z"
  }
}
"#
        );

        let path = std::env::temp_dir().join("hotpath_cloud_cli_auth_output.json");
        let _ = std::fs::remove_file(&path);
        let output = hotpath(
            &server,
            Some(TOKEN),
            &["auth", "--output", path.to_str().unwrap()],
        );
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(stdout(&output), "");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{AUTH_BODY}\n")
        );
        let _ = std::fs::remove_file(&path);

        mock.assert();
    }

    #[test]
    fn auth_without_a_token_does_not_call_the_server() {
        let mut server = Server::new();
        let mock = server.mock("GET", "/api/v1/auth").expect(0).create();

        for token in [None, Some(""), Some("   ")] {
            let output = hotpath(&server, token, &["auth"]);
            assert_eq!(output.status.code(), Some(1));
            assert_eq!(stdout(&output), "");
            assert_eq!(
                client_error(&output),
                "HOTPATH_API_TOKEN is not set. Create a token at https://hotpath.rs/app/tokens and export it."
            );
        }
        mock.assert();
    }

    #[test]
    fn auth_rejected_token_prints_the_server_body_verbatim() {
        let mut server = Server::new();
        // Key order the client would not produce itself, so a byte-for-byte
        // match proves the body is not re-serialized.
        let body = r#"{"error":"The token is unknown, expired or revoked. Create one at /app/tokens.","code":"invalid_token"}"#;
        let mock = server
            .mock("GET", "/api/v1/auth")
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_header("www-authenticate", "Bearer")
            .with_header("x-request-id", "1bac4db9-15a")
            .with_body(body)
            .create();

        let output = hotpath(&server, Some(TOKEN), &["auth"]);
        mock.assert();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        assert_eq!(stderr(&output), format!("{body}\n"));
    }

    #[test]
    fn auth_server_error_passes_through_without_client_hints() {
        // Status, body: every body prints as sent, with no client hint appended.
        let cases: [(u16, String); 4] = [
            (
                401,
                error_body(
                    ApiErrorCode::GithubAuthorizationExpired,
                    "Your GitHub authorization expired.",
                ),
            ),
            (
                429,
                error_body(ApiErrorCode::RateLimited, "Too many requests."),
            ),
            (500, error_body(ApiErrorCode::Internal, "Something broke.")),
            (
                401,
                r#"{"error":"A code this client does not know.","code":"quota_exceeded"}"#.into(),
            ),
        ];
        for (status, body) in cases {
            let mut server = Server::new();
            let mock = server
                .mock("GET", "/api/v1/auth")
                .with_status(status.into())
                .with_header("content-type", "application/json")
                .with_header("retry-after", "30")
                .with_header("x-request-id", "abc")
                .with_body(&body)
                .create();

            let output = hotpath(&server, Some(TOKEN), &["auth"]);
            mock.assert();
            assert_eq!(output.status.code(), Some(1), "{body}");
            assert_eq!(stdout(&output), "", "{body}");
            assert_eq!(stderr(&output), format!("{body}\n"));
        }
    }

    #[test]
    fn auth_pretty_indents_the_server_error() {
        let mut server = Server::new();
        let body = error_body(ApiErrorCode::InvalidToken, "Nope.");
        let mock = server
            .mock("GET", "/api/v1/auth")
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create();

        let output = hotpath(&server, Some(TOKEN), &["auth", "--pretty"]);
        mock.assert();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        let expected: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            stderr(&output),
            format!("{}\n", serde_json::to_string_pretty(&expected).unwrap())
        );
    }

    #[test]
    fn auth_non_json_error_quotes_status_and_body_prefix() {
        let mut server = Server::new();
        let page = format!("<html>{}</html>", "x".repeat(400));
        let mock = server
            .mock("GET", "/api/v1/auth")
            .with_status(502)
            .with_header("content-type", "text/html")
            .with_body(&page)
            .create();

        let output = hotpath(&server, Some(TOKEN), &["auth"]);
        mock.assert();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        assert_eq!(
            client_error(&output),
            format!("HTTP 502: {}...", &page[..200])
        );
    }

    #[test]
    fn auth_non_json_success_body_is_an_error() {
        let mut server = Server::new();
        let mock = server
            .mock("GET", "/api/v1/auth")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"login":"pawurb""#)
            .expect(2)
            .create();

        for args in [&["auth"][..], &["auth", "--pretty"]] {
            let output = hotpath(&server, Some(TOKEN), args);
            assert_eq!(output.status.code(), Some(1));
            assert_eq!(stdout(&output), "");
            let error = client_error(&output);
            assert!(error.starts_with("invalid response from "), "{error}");
        }
        mock.assert();
    }

    #[test]
    fn auth_unreachable_server_is_exit_1() {
        // A port nothing listens on: bind, read it back, release it.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        drop(listener);

        let output = Command::new(env!("CARGO_BIN_EXE_hotpath"))
            .args(["cloud", "auth"])
            .env("HOTPATH_API_URL", &url)
            .env("HOTPATH_API_TOKEN", TOKEN)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        let error = client_error(&output);
        assert!(
            error.starts_with(&format!("request to {url} failed: ")),
            "{error}"
        );
        assert!(!error.contains(TOKEN), "{error}");
    }

    #[test]
    fn repos_prints_the_list_compact_and_pretty() {
        let mut server = Server::new();
        let mock = mock_get(&mut server, "/api/v1/repos", REPOS_BODY).expect(2);

        let output = hotpath(&server, Some(TOKEN), &["repos"]);
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(stdout(&output), format!("{REPOS_BODY}\n"));
        assert_eq!(stderr(&output), "");

        let output = hotpath(&server, Some(TOKEN), &["repos", "--pretty"]);
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        let printed = stdout(&output);
        assert!(
            printed.starts_with("{\n  \"repositories\": [\n"),
            "{printed}"
        );
        assert_eq!(json(&printed), json(REPOS_BODY));

        mock.assert();
    }

    #[test]
    fn benchmarks_requests_the_repo_flag_repository() {
        let mut server = Server::new();
        let mock = mock_get(&mut server, BENCHMARKS_PATH, BENCHMARKS_BODY);

        let output = hotpath(
            &server,
            Some(TOKEN),
            &["benchmarks", "--repo", "hotpath-rs/hotpath"],
        );
        mock.assert();
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(stdout(&output), format!("{BENCHMARKS_BODY}\n"));
        assert_eq!(stderr(&output), "");
    }

    #[test]
    fn benchmarks_rejects_a_missing_or_bad_repo_flag_without_a_request() {
        let mut server = Server::new();
        let mock = server
            .mock("GET", Matcher::Regex("^/api/v1/repos/".into()))
            .expect(0)
            .create();

        let output = hotpath(&server, Some(TOKEN), &["benchmarks"]);
        assert_eq!(output.status.code(), Some(2), "clap usage error");
        assert_eq!(stdout(&output), "");
        assert!(stderr(&output).contains("--repo"), "{}", stderr(&output));

        for value in [
            "nope", "a/b/c", "a/..", "./b", "a/", "/b", "a b/c", "a/b?x=1",
        ] {
            // No token either: the bad argument is what gets reported.
            for token in [Some(TOKEN), None] {
                let output = hotpath(&server, token, &["benchmarks", "--repo", value]);
                assert_eq!(output.status.code(), Some(1), "{value}");
                assert_eq!(stdout(&output), "", "{value}");
                let error = client_error(&output);
                assert!(error.contains("invalid --repo"), "{value}: {error}");
                assert!(error.contains(value), "{value}: {error}");
            }
        }
        mock.assert();
    }

    #[test]
    fn benchmarks_errors_print_the_server_body_verbatim() {
        // The two codes `auth` never produces: an unknown, invisible or
        // App-less repository, and a lapsed GitHub authorization.
        let cases: [(u16, String); 2] = [
            (
                404,
                error_body(
                    ApiErrorCode::NotFound,
                    "Repository hotpath-rs/hotpath not found.",
                ),
            ),
            (
                401,
                error_body(
                    ApiErrorCode::GithubAuthorizationExpired,
                    "Your GitHub authorization expired. Log in at https://hotpath.rs/app once.",
                ),
            ),
        ];
        for (status, body) in cases {
            let mut server = Server::new();
            let mock = server
                .mock("GET", BENCHMARKS_PATH)
                .with_status(status.into())
                .with_header("content-type", "application/json")
                .with_header("x-request-id", "req-1")
                .with_body(&body)
                .create();

            let output = hotpath(
                &server,
                Some(TOKEN),
                &["benchmarks", "--repo", "hotpath-rs/hotpath"],
            );
            mock.assert();
            assert_eq!(output.status.code(), Some(1), "{body}");
            assert_eq!(stdout(&output), "", "{body}");
            assert_eq!(stderr(&output), format!("{body}\n"));
        }
    }

    #[test]
    fn report_by_pr_prints_the_report_compact_and_pretty() {
        let mut server = Server::new();
        let body = report_body();
        let mock = mock_report(
            &mut server,
            &format!("{REPORTS_PATH}/latest"),
            "pr=105",
            &body,
        )
        .expect(2);

        let output = hotpath(&server, Some(TOKEN), &report_args(&["--pr", "105"]));
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(stderr(&output), "");
        let printed = stdout(&output);
        assert_eq!(printed, format!("{body}\n"));
        assert!(printed.contains(r#""git_ref":null"#), "{printed}");
        // The policy is named by its path, the document is never returned.
        assert!(
            printed.contains(r#""policy_path":"hotpath/ci-policy.toml""#),
            "{printed}"
        );
        assert!(
            printed.contains(r#""policy_url":"https://github.com/hotpath-rs/hotpath/blob/3f1c000000000000000000000000000000000000/hotpath/ci-policy.toml""#),
            "{printed}"
        );
        assert!(!printed.contains(r#""policy":"#), "{printed}");

        let output = hotpath(
            &server,
            Some(TOKEN),
            &report_args(&["--pr", "105", "--pretty"]),
        );
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        let printed = stdout(&output);
        assert_eq!(json(&printed), json(&body));
        // The server's key order: the payload last, as the writer ordered it.
        assert!(printed.starts_with("{\n  \"id\": "), "{printed}");
        assert!(
            printed.ends_with(
                "\n  \"payload\": {\n    \"version\": \"0.26.1\",\n    \"meta\": {}\n  }\n}\n"
            ),
            "{printed}"
        );

        mock.assert();
    }

    #[test]
    fn report_by_commit_sends_the_event_and_lowercases_the_sha() {
        let mut server = Server::new();
        let body = report_body();
        let mock = mock_report(
            &mut server,
            &format!("{REPORTS_PATH}/latest"),
            &format!("commit={SHA}&event=push"),
            &body,
        )
        .expect(2);

        for sha in [SHA.to_string(), SHA.to_ascii_uppercase()] {
            let output = hotpath(
                &server,
                Some(TOKEN),
                &report_args(&["--commit", &sha, "--event", "push"]),
            );
            assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
            assert_eq!(json(&stdout(&output)), json(&body));
        }
        mock.assert();
    }

    #[test]
    fn report_by_id_without_payload_prints_the_summary() {
        let mut server = Server::new();
        let mock = mock_report(
            &mut server,
            &format!("{REPORTS_PATH}/{REPORT_ID}"),
            "payload=false",
            SUMMARY_BODY,
        );

        let output = hotpath(
            &server,
            Some(TOKEN),
            &report_args(&["--id", REPORT_ID, "--no-payload"]),
        );
        mock.assert();
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(stdout(&output), format!("{SUMMARY_BODY}\n"));
        assert_eq!(json(&stdout(&output))["id"], REPORT_ID);
    }

    #[test]
    fn report_not_uploaded_yet_is_the_server_404_verbatim() {
        let mut server = Server::new();
        let body = error_body(
            ApiErrorCode::NotFound,
            "No report of benchmark ci matches that commit.",
        );
        let mock = server
            .mock("GET", format!("{REPORTS_PATH}/latest").as_str())
            .match_query(Matcher::Exact(format!("commit={SHA}")))
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create();

        let output = hotpath(&server, Some(TOKEN), &report_args(&["--commit", SHA]));
        mock.assert();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        assert_eq!(stderr(&output), format!("{body}\n"));
    }

    #[test]
    fn report_rejects_bad_values_before_the_token_or_a_request() {
        let mut server = Server::new();
        let mock = server
            .mock("GET", Matcher::Regex("^/api/v1/".into()))
            .expect(0)
            .create();

        let long_name = "a".repeat(65);
        let non_hex = "g".repeat(40);
        let cases: [(Vec<&str>, &str); 9] = [
            (
                report_args(&["--commit", "9ab2"]),
                "invalid --commit `9ab2`",
            ),
            (report_args(&["--commit", &non_hex]), "invalid --commit"),
            (report_args(&["--pr", "0"]), "invalid --pr `0`"),
            (
                report_args(&["--id", "0199a3c2/../../x"]),
                "invalid --id `0199a3c2/../../x`",
            ),
            (
                vec![
                    "report",
                    "--repo",
                    "hotpath-rs/hotpath",
                    "--benchmark",
                    "a/b",
                    "--pr",
                    "1",
                ],
                "invalid --benchmark `a/b`",
            ),
            (
                vec![
                    "report",
                    "--repo",
                    "hotpath-rs/hotpath",
                    "--benchmark",
                    "..",
                    "--pr",
                    "1",
                ],
                "invalid --benchmark `..`",
            ),
            (
                vec![
                    "report",
                    "--repo",
                    "hotpath-rs/hotpath",
                    "--benchmark",
                    &long_name,
                    "--pr",
                    "1",
                ],
                "invalid --benchmark",
            ),
            (
                vec!["report", "--repo", "nope", "--benchmark", "ci", "--pr", "1"],
                "invalid --repo `nope`",
            ),
            (
                vec!["report", "--repo", "a/..", "--benchmark", "ci", "--pr", "1"],
                "invalid --repo `a/..`",
            ),
        ];
        for (args, expected) in cases {
            let output = hotpath(&server, None, &args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert_eq!(stdout(&output), "", "{args:?}");
            let error = client_error(&output);
            assert!(error.starts_with(expected), "{args:?}: {error}");
        }
        assert!(
            client_error(&hotpath(&server, None, &report_args(&["--commit", "9ab2"])))
                .contains("git rev-parse HEAD")
        );
        mock.assert();
    }

    #[test]
    fn report_selector_misuse_is_a_clap_usage_error() {
        let mut server = Server::new();
        let mock = server
            .mock("GET", Matcher::Regex("^/api/v1/".into()))
            .expect(0)
            .create();

        for selector in [
            vec![],
            vec!["--pr", "1", "--commit", SHA],
            vec!["--pr", "1", "--id", REPORT_ID],
            vec!["--id", REPORT_ID, "--event", "push"],
            vec!["--pr", "1", "--event", "merge"],
            vec!["--pr", "-1"],
        ] {
            let output = hotpath(&server, Some(TOKEN), &report_args(&selector));
            assert_eq!(
                output.status.code(),
                Some(2),
                "{selector:?}: {}",
                stderr(&output)
            );
            assert_eq!(stdout(&output), "", "{selector:?}");
        }
        mock.assert();
    }

    const DIFF_PATH: &str = "/api/v1/repos/hotpath-rs/hotpath/benchmarks/ci/diff";
    const BASE_ID: &str = "0199a3b0-0000-7000-8000-000000000000";

    const NO_BASELINE: &str = r#"{"status":"no_baseline"}"#;
    const UNREADABLE_BASE: &str = r#"{"status":"unreadable","side":"base","hotpath_version":"0.20.0","error":"missing field `functions_timing`"}"#;
    const UNREADABLE_HEAD: &str = r#"{"status":"unreadable","side":"head","hotpath_version":null,"error":"missing field `functions_timing`"}"#;

    /// What the policy's budgets came to on head.
    #[derive(Clone, Copy, Debug)]
    enum BudgetsCase {
        /// Head does not parse: `budgets` is `null`.
        Unread,
        /// The policy has no budget rules.
        NoRules,
        /// One rule, which holds (no finding under `rows=findings`).
        Hold,
        /// One rule, broken.
        Broken,
    }

    impl BudgetsCase {
        fn broken(self) -> u64 {
            match self {
                BudgetsCase::Broken => 1,
                BudgetsCase::Unread | BudgetsCase::NoRules | BudgetsCase::Hold => 0,
            }
        }

        fn has_rules(self) -> bool {
            matches!(self, BudgetsCase::Hold | BudgetsCase::Broken)
        }

        fn body(self) -> String {
            let findings = match self {
                BudgetsCase::Unread => return "null".to_string(),
                BudgetsCase::Broken => {
                    r#"{"resource":"functions","rule":0,"pattern":"app::run","message":"run must stay under 1 ms","entity":{"key":"app::run","name":"app::run","location":null},"check":{"on":"column","family":"timing","kind":"timing","column":"p95"},"bound":"max","unit":"duration","limit":"1.00 ms","actual":"1.50 ms","broken":true}"#
                }
                BudgetsCase::NoRules | BudgetsCase::Hold => "",
            };
            format!(
                r#"{{"rules":{},"broken":{},"findings":[{findings}],"notes":[]}}"#,
                u64::from(self.has_rules()),
                self.broken(),
            )
        }
    }

    /// A `ReportDiff` body with `SUMMARY_BODY` as head (and as base, which
    /// the client never checks), `result` as given and the verdict the
    /// server builds from `result` and `budgets`.
    fn diff_body(result: &str, budgets: BudgetsCase) -> String {
        let base = if result == NO_BASELINE {
            "null".to_string()
        } else {
            format!(r#"{{"report":{SUMMARY_BODY},"branch_point":true}}"#)
        };
        let was_compared = result.contains(r#""status":"compared""#);
        let regressions = u64::from(result.contains(r#""outcome":"regression""#));
        let budgets_broken = budgets.broken();
        let judged = was_compared || budgets.has_rules();
        let regressed = regressions > 0 || budgets_broken > 0;
        format!(
            r#"{{"repository":"hotpath-rs/hotpath","benchmark":"ci","head":{SUMMARY_BODY},"base":{base},"rows":"findings","verdict":{{"judged":{judged},"regressed":{regressed},"regressions":{regressions},"improvements":0,"budgets_broken":{budgets_broken}}},"budgets":{budgets},"result":{result},"dashboard_url":"https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/ci/reports/0199a3c2-7d2e-7b41-9c3a-1f2e3d4c5b6a/diff"}}"#,
            budgets = budgets.body(),
        )
    }

    /// A `compared` result with one judged `functions` timing row.
    fn compared(regressed: bool) -> String {
        let (outcome, crossed, regressions) = if regressed {
            ("regression", r#","crossed":"up""#, 1)
        } else {
            ("unchanged", "", 0)
        };
        format!(
            r#"{{"status":"compared","totals":{{"elapsed":null,"allocated":null,"peak_rss":null}},"sections":[{{"resource":"functions","kind":"timing","mode":"timing","base_coverage":{{"included":1,"total":1}},"head_coverage":{{"included":1,"total":1}},"family":{{"name":"timing","judged":true,"min_percent_change":10.0,"metrics":["p95"],"counts":{{"ignored":0,"below_floor":0,"added":0,"removed":0,"too_few_calls":0,"regressions":{regressions},"improvements":0,"unchanged":{unchanged}}}}},"rows":[{{"key":"app::run","name":"app::run","presence":"both","outcome":"{outcome}","cells":[{{"column":"p95","unit":"duration","base":"1.00 µs","head":"1.50 µs","change_percent":50.0{crossed}}}]}}]}}],"skipped":[],"notes":[]}}"#,
            unchanged = 1 - regressions,
        )
    }

    fn diff_args<'a>(selector: &[&'a str]) -> Vec<&'a str> {
        let mut args = vec!["diff", "--repo", "hotpath-rs/hotpath", "--benchmark", "ci"];
        args.extend_from_slice(selector);
        args
    }

    /// Runs `diff --pr 42` against a 200 with `body`; the exit code is read
    /// from the body, so every status prints it on stdout.
    fn diff_by_pr(body: &str) -> Output {
        let mut server = Server::new();
        let mock = mock_report(&mut server, DIFF_PATH, "pr=42", body);
        let output = hotpath(&server, Some(TOKEN), &diff_args(&["--pr", "42"]));
        mock.assert();
        output
    }

    #[test]
    fn diff_exit_code_follows_the_verdict() {
        for (result, budgets, code) in [
            (compared(false), BudgetsCase::Hold, 0),
            (compared(false), BudgetsCase::NoRules, 0),
            (compared(false), BudgetsCase::Broken, 1),
            (compared(true), BudgetsCase::Hold, 1),
            (NO_BASELINE.to_string(), BudgetsCase::Hold, 0),
            (NO_BASELINE.to_string(), BudgetsCase::Broken, 1),
            (NO_BASELINE.to_string(), BudgetsCase::NoRules, 1),
            (UNREADABLE_BASE.to_string(), BudgetsCase::Hold, 0),
            (UNREADABLE_BASE.to_string(), BudgetsCase::NoRules, 1),
            (UNREADABLE_HEAD.to_string(), BudgetsCase::Unread, 1),
        ] {
            let body = diff_body(&result, budgets);
            let output = diff_by_pr(&body);
            let case = format!("{budgets:?} {result}");
            assert_eq!(output.status.code(), Some(code), "{case}");
            assert_eq!(stderr(&output), "", "{case}");
            assert_eq!(stdout(&output), format!("{body}\n"), "{case}");
        }
    }

    #[test]
    fn diff_passes_unknown_fields_through_and_still_exits_by_the_verdict() {
        for (budgets, code) in [(BudgetsCase::Hold, 0), (BudgetsCase::Broken, 1)] {
            let body = diff_body(&compared(false), budgets);
            let body = format!(r#"{{"new_field":{{"z":1,"a":2}},{}"#, &body[1..]);
            let output = diff_by_pr(&body);
            assert_eq!(output.status.code(), Some(code), "{budgets:?}");
            assert_eq!(stderr(&output), "", "{budgets:?}");
            assert_eq!(stdout(&output), format!("{body}\n"), "{budgets:?}");
        }

        let body = diff_body(&compared(false), BudgetsCase::Hold);
        let body = format!(r#"{{"new_field":{{"z":1,"a":2}},{}"#, &body[1..]);
        let mut server = Server::new();
        let mock = mock_report(&mut server, DIFF_PATH, "pr=42", &body);
        let output = hotpath(
            &server,
            Some(TOKEN),
            &diff_args(&["--pr", "42", "--pretty"]),
        );
        mock.assert();
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        let printed = stdout(&output);
        assert!(
            printed.starts_with(
                "{\n  \"new_field\": {\n    \"z\": 1,\n    \"a\": 2\n  },\n  \"repository\": "
            ),
            "{printed}"
        );
        assert_eq!(json(&printed), json(&body));
    }

    #[test]
    fn diff_sends_commit_event_and_full_and_maps_id_to_head() {
        let mut server = Server::new();
        let body = diff_body(&compared(false), BudgetsCase::NoRules);
        let by_commit = server
            .mock("GET", DIFF_PATH)
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("commit".into(), SHA.into()),
                Matcher::UrlEncoded("event".into(), "pull_request".into()),
                Matcher::UrlEncoded("rows".into(), "all".into()),
            ]))
            .match_header("authorization", format!("Bearer {TOKEN}").as_str())
            .with_status(200)
            .with_body(&body)
            .create();
        let advisory = mock_report(&mut server, DIFF_PATH, "pr=42&rows=advisory", &body);
        // Without `--full` the query leaves `rows=` to the server's default.
        let by_id = mock_report(&mut server, DIFF_PATH, &format!("head={REPORT_ID}"), &body);

        let output = hotpath(
            &server,
            Some(TOKEN),
            &diff_args(&[
                "--commit",
                &SHA.to_ascii_uppercase(),
                "--event",
                "pull_request",
                "--full",
            ]),
        );
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        let output = hotpath(&server, Some(TOKEN), &diff_args(&["--id", REPORT_ID]));
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        let output = hotpath(
            &server,
            Some(TOKEN),
            &diff_args(&["--pr", "42", "--advisory"]),
        );
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        by_commit.assert();
        advisory.assert();
        by_id.assert();

        let output = hotpath(
            &server,
            Some(TOKEN),
            &diff_args(&["--pr", "42", "--advisory", "--full"]),
        );
        assert_eq!(output.status.code(), Some(2), "clap usage error");
    }

    #[test]
    fn diff_unknown_report_is_the_server_404_verbatim() {
        let mut server = Server::new();
        let body = error_body(ApiErrorCode::NotFound, "No report of benchmark ci matches.");
        let mock = server
            .mock("GET", DIFF_PATH)
            .match_query(Matcher::Exact("pr=42".into()))
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create();

        let output = hotpath(&server, Some(TOKEN), &diff_args(&["--pr", "42"]));
        mock.assert();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        assert_eq!(stderr(&output), format!("{body}\n"));
    }

    #[test]
    fn diff_body_without_a_verdict_is_an_error_not_a_pass() {
        let body = diff_body(&compared(false), BudgetsCase::NoRules);
        let mut value = json(&body);
        value.as_object_mut().unwrap().remove("verdict");
        let output = diff_by_pr(&value.to_string());
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        let error = client_error(&output);
        assert!(error.starts_with("invalid response from"), "{error}");
        assert!(error.contains("verdict"), "{error}");
    }

    #[test]
    fn diff_body_without_a_result_still_exits_by_the_verdict() {
        // The rest of the body is the server's: only the verdict is read.
        let mut value = json(&diff_body(&compared(false), BudgetsCase::NoRules));
        value.as_object_mut().unwrap().remove("result");
        let body = value.to_string();
        let output = diff_by_pr(&body);
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(stdout(&output), format!("{body}\n"));
    }

    #[test]
    fn diff_body_with_the_verdict_inside_compared_is_an_error_not_a_pass() {
        let mut value = json(&diff_body(&compared(false), BudgetsCase::NoRules));
        let verdict = value.as_object_mut().unwrap().remove("verdict").unwrap();
        value["result"]["verdict"] = verdict;
        let output = diff_by_pr(&value.to_string());
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        let error = client_error(&output);
        assert!(error.starts_with("invalid response from"), "{error}");
        assert!(error.contains("verdict"), "{error}");
    }

    #[test]
    fn diff_rejects_bad_values_before_the_token_or_a_request() {
        let mut server = Server::new();
        let mock = server
            .mock("GET", Matcher::Regex("^/api/v1/".into()))
            .expect(0)
            .create();

        let cases: [(Vec<&str>, &str); 3] = [
            (
                vec!["diff", "--repo", "nope", "--benchmark", "ci", "--pr", "1"],
                "invalid --repo `nope`",
            ),
            (
                vec![
                    "diff",
                    "--repo",
                    "hotpath-rs/hotpath",
                    "--benchmark",
                    "a/b",
                    "--pr",
                    "1",
                ],
                "invalid --benchmark `a/b`",
            ),
            (diff_args(&["--commit", "9ab2"]), "invalid --commit `9ab2`"),
        ];
        for (args, expected) in cases {
            let output = hotpath(&server, None, &args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert_eq!(stdout(&output), "", "{args:?}");
            let error = client_error(&output);
            assert!(error.starts_with(expected), "{args:?}: {error}");
        }
        mock.assert();
    }

    #[test]
    fn diff_selector_misuse_is_a_clap_usage_error() {
        let mut server = Server::new();
        let mock = server
            .mock("GET", Matcher::Regex("^/api/v1/".into()))
            .expect(0)
            .create();

        for selector in [
            vec![],
            vec!["--pr", "1", "--base", BASE_ID],
            vec!["--id", REPORT_ID, "--event", "push"],
            vec!["--pr", "1", "--commit", SHA],
        ] {
            let output = hotpath(&server, Some(TOKEN), &diff_args(&selector));
            assert_eq!(
                output.status.code(),
                Some(2),
                "{selector:?}: {}",
                stderr(&output)
            );
            assert_eq!(stdout(&output), "", "{selector:?}");
        }
        mock.assert();
    }

    #[test]
    fn removed_policy_commands_are_usage_errors() {
        let mut server = Server::new();
        let mock = server
            .mock("GET", Matcher::Regex("^/api/v1/".into()))
            .expect(0)
            .create();

        for args in [
            // Gone: the policy is a file in the repository, read per report.
            vec!["set-policy", "--repo", "hotpath-rs/hotpath", "--file", "x"],
            vec![
                "get-policy",
                "--repo",
                "hotpath-rs/hotpath",
                "--benchmark",
                "ci",
            ],
        ] {
            let output = hotpath(&server, Some(TOKEN), &args);
            assert_eq!(
                output.status.code(),
                Some(2),
                "{args:?}: {}",
                stderr(&output)
            );
            assert_eq!(stdout(&output), "", "{args:?}");
        }
        mock.assert();
    }

    const VALIDATE_PATH: &str = "/api/v1/policy/validate";
    const GOOD_POLICY: &str = "[functions.timing]\nmin_percent_change = 5\n";
    const BAD_POLICY: &str = "[functions.timing]\nmin_percent = 5\n";
    const REJECTED_BODY: &str = r#"{"error":"The policy has 1 problem.","code":"invalid_policy","problems":[{"line":2,"message":"unknown key `functions.timing.min_percent`"}]}"#;
    const REJECTED_PROBLEMS: &str =
        r#"[{"line":2,"message":"unknown key `functions.timing.min_percent`"}]"#;

    /// A directory that stands for a checkout: it has a `.git` and whatever
    /// files a test writes. Removed on drop, so a failed test leaves nothing.
    struct Checkout {
        root: PathBuf,
    }

    impl Checkout {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("hotpath_cloud_cli_{}_{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join(".git")).unwrap();
            Self { root }
        }

        /// A directory without a `.git` anywhere above what the test made.
        fn without_git(name: &str) -> Self {
            let checkout = Self::new(name);
            std::fs::remove_dir(checkout.root.join(".git")).unwrap();
            checkout
        }

        fn write(&self, path: &str, contents: &[u8]) -> &Self {
            let path = self.root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
            self
        }

        fn validate(&self, server: &ServerGuard, token: Option<&str>, args: &[&str]) -> Output {
            let mut all = vec!["validate-policy"];
            all.extend_from_slice(args);
            hotpath_in(server, token, &all, b"", Some(&self.root), None)
        }

        /// `init` run from a directory below the root, which is found as the
        /// git root above it.
        fn init(&self, server: &ServerGuard, token: Option<&str>, args: &[&str]) -> Output {
            let cwd = self.root.join("src");
            std::fs::create_dir_all(&cwd).unwrap();
            let mut all = vec!["init"];
            all.extend_from_slice(args);
            hotpath_in(server, token, &all, b"", Some(&cwd), None)
        }

        fn read(&self, path: &str) -> Option<String> {
            std::fs::read_to_string(self.root.join(path)).ok()
        }
    }

    impl Drop for Checkout {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// A mock of the validation route for exactly this document. The route
    /// is public: a request carrying any `Authorization` header does not match.
    fn mock_validate(
        server: &mut ServerGuard,
        source: &str,
        status: usize,
        body: &str,
    ) -> mockito::Mock {
        server
            .mock("POST", VALIDATE_PATH)
            .match_header("authorization", Matcher::Missing)
            .match_header("content-type", "application/json")
            .match_header("user-agent", Matcher::Regex("^hotpath-cli/[0-9]".into()))
            .match_body(Matcher::Json(serde_json::json!({ "source": source })))
            .with_status(status)
            .with_header("content-type", "application/json; charset=utf-8")
            .with_body(body)
            .create()
    }

    fn mock_no_validation(server: &mut ServerGuard) -> mockito::Mock {
        server
            .mock("POST", Matcher::Regex("^/api/v1/".into()))
            .expect(0)
            .create()
    }

    #[test]
    fn validate_policy_without_arguments_checks_the_shared_file_only() {
        let mut server = Server::new();
        let good = mock_validate(&mut server, GOOD_POLICY, 200, r#"{"valid":true}"#);
        let bad = mock_validate(&mut server, BAD_POLICY, 422, REJECTED_BODY).expect(0);
        let checkout = Checkout::new("shared_file");
        checkout
            .write("hotpath/policy.toml", GOOD_POLICY.as_bytes())
            // Files of other benchmarks, and of a mistyped one, are not sent.
            .write("hotpath/ci-policy.toml", BAD_POLICY.as_bytes())
            .write("hotpath/blank-policy.toml", b" \n\t\n");

        let output = checkout.validate(&server, None, &[]);
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(
            stdout(&output),
            "{\"path\":\"hotpath/policy.toml\",\"valid\":true,\"problems\":[]}\n"
        );
        assert_eq!(stderr(&output), "");
        good.assert();
        bad.assert();
    }

    #[test]
    fn validate_policy_without_arguments_checks_the_override() {
        let mut server = Server::new();
        let good = mock_validate(&mut server, GOOD_POLICY, 200, r#"{"valid":true}"#).expect(0);
        let bad = mock_validate(&mut server, BAD_POLICY, 422, REJECTED_BODY);
        let checkout = Checkout::new("override");
        checkout
            .write("hotpath/policy.toml", GOOD_POLICY.as_bytes())
            .write("config/special.toml", BAD_POLICY.as_bytes());

        let output = hotpath_in(
            &server,
            None,
            &["validate-policy"],
            b"",
            Some(&checkout.root),
            Some("config/special.toml"),
        );
        assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
        assert_eq!(stderr(&output), "");
        assert_eq!(
            json(&stdout(&output)),
            json(&format!(
                r#"{{"path":"config/special.toml","valid":false,"problems":{REJECTED_PROBLEMS}}}"#
            ))
        );
        good.assert();
        bad.assert();
    }

    #[test]
    fn validate_policy_benchmark_checks_the_file_a_run_would_pick() {
        let mut server = Server::new();
        let good = mock_validate(&mut server, GOOD_POLICY, 200, r#"{"valid":true}"#).expect(2);
        let bad = mock_validate(&mut server, BAD_POLICY, 422, REJECTED_BODY);
        let checkout = Checkout::new("benchmark");
        checkout
            .write("hotpath/policy.toml", GOOD_POLICY.as_bytes())
            .write("hotpath/ci-policy.toml", BAD_POLICY.as_bytes())
            .write("config/special.toml", GOOD_POLICY.as_bytes());

        // Its own file wins over the shared one.
        let output = checkout.validate(&server, None, &["--benchmark", "ci"]);
        assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
        assert_eq!(
            json(&stdout(&output)),
            json(&format!(
                r#"{{"path":"hotpath/ci-policy.toml","valid":false,"problems":{REJECTED_PROBLEMS}}}"#
            ))
        );

        // A benchmark without its own file uses the shared one.
        let output = checkout.validate(&server, None, &["--benchmark", "nightly"]);
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(
            json(&stdout(&output)),
            json(r#"{"path":"hotpath/policy.toml","valid":true,"problems":[]}"#)
        );

        // `HOTPATH_POLICY_PATH` wins over both, as it does for the run.
        let output = hotpath_in(
            &server,
            None,
            &["validate-policy", "--benchmark", "ci"],
            b"",
            Some(&checkout.root),
            Some("config/special.toml"),
        );
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(
            json(&stdout(&output)),
            json(r#"{"path":"config/special.toml","valid":true,"problems":[]}"#)
        );

        good.assert();
        bad.assert();
    }

    #[test]
    fn validate_policy_file_checks_a_file_anywhere_and_stdin() {
        let mut server = Server::new();
        let mock = mock_validate(&mut server, GOOD_POLICY, 200, r#"{"valid":true}"#).expect(2);
        // No checkout is needed for a file named on the command line.
        let outside = Checkout::without_git("file_anywhere");
        outside.write("candidate.toml", GOOD_POLICY.as_bytes());
        let file = outside.root.join("candidate.toml");

        let output = outside.validate(
            &server,
            None,
            &["--file", file.to_str().unwrap(), "--pretty"],
        );
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(
            json(&stdout(&output)),
            serde_json::json!({ "path": file.to_str().unwrap(), "valid": true, "problems": [] })
        );
        assert!(stdout(&output).starts_with("{\n  \""), "pretty");

        let output = hotpath_with_stdin(
            &server,
            None,
            &["validate-policy", "--file", "-"],
            GOOD_POLICY.as_bytes(),
        );
        assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
        assert_eq!(
            json(&stdout(&output)),
            json(r#"{"path":"-","valid":true,"problems":[]}"#)
        );
        mock.assert();
    }

    #[test]
    fn validate_policy_refuses_unusable_files_without_a_request() {
        let mut server = Server::new();
        let mock = mock_no_validation(&mut server);
        let checkout = Checkout::new("unusable");
        checkout
            .write("hotpath/blank-policy.toml", b"\n")
            .write(
                "hotpath/large-policy.toml",
                &vec![b'#'; POLICY_MAX_BYTES + 1],
            )
            .write("hotpath/latin-policy.toml", b"name = \"\xff\xfe\"\n");
        let outside = Checkout::without_git("unusable_outside");
        outside.write("policy.toml", GOOD_POLICY.as_bytes());

        let expected = [
            (
                "blank",
                "the policy file `hotpath/blank-policy.toml` is blank.",
            ),
            (
                "large",
                "the policy file `hotpath/large-policy.toml` is larger than 65536 bytes",
            ),
            (
                "latin",
                "the policy file `hotpath/latin-policy.toml` is not valid UTF-8",
            ),
        ];
        for (benchmark, message) in expected {
            let output = checkout.validate(&server, None, &["--benchmark", benchmark]);
            assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
            assert_eq!(stderr(&output), "");
            let file = json(&stdout(&output));
            assert_eq!(file["path"], format!("hotpath/{benchmark}-policy.toml"));
            assert_eq!(file["valid"], false);
            assert_eq!(file["problems"].as_array().unwrap().len(), 1, "{file}");
            assert_eq!(file["problems"][0]["line"], serde_json::Value::Null);
            let found = file["problems"][0]["message"].as_str().unwrap();
            assert!(found.starts_with(message), "{found}");
        }

        // An override that leads out of the repository, and one that is missing.
        for (policy_path, message) in [
            (
                outside.root.join("policy.toml"),
                "is outside the repository",
            ),
            (checkout.root.join("hotpath/gone.toml"), "could not read"),
        ] {
            let output = hotpath_in(
                &server,
                None,
                &["validate-policy", "--benchmark", "ci"],
                b"",
                Some(&checkout.root),
                Some(policy_path.to_str().unwrap()),
            );
            assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
            let file = json(&stdout(&output));
            assert_eq!(file["valid"], false, "{file}");
            let found = file["problems"][0]["message"].as_str().unwrap();
            assert!(found.contains(message), "{found}");
            assert!(found.contains("HOTPATH_POLICY_PATH"), "{found}");
        }

        // A link out of the repository is followed before the check.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.root.join("policy.toml"),
                checkout.root.join("hotpath/linked-policy.toml"),
            )
            .unwrap();
            let output = checkout.validate(&server, None, &["--benchmark", "linked"]);
            assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
            let file = json(&stdout(&output));
            let found = file["problems"][0]["message"].as_str().unwrap();
            assert!(found.contains("is outside the repository"), "{found}");
        }

        mock.assert();
    }

    #[test]
    fn validate_policy_without_a_file_to_check_is_an_error() {
        let mut server = Server::new();
        let mock = mock_no_validation(&mut server);
        // Only a benchmark's own file: the repository has a policy, but no
        // shared one.
        let unshared = Checkout::new("no_file");
        unshared
            .write("hotpath/ci-policy.toml", GOOD_POLICY.as_bytes())
            .write("hotpath/notes.toml", b"x");
        let outside = Checkout::without_git("no_checkout");

        let cases: [(&Checkout, &[&str], &str); 7] = [
            (&unshared, &[], "no shared policy file in "),
            (
                &unshared,
                &[],
                "so hotpath.rs refuses the reports of every benchmark",
            ),
            (&unshared, &[], "with --benchmark NAME"),
            (
                &unshared,
                &["--benchmark", "nightly"],
                "no policy file for benchmark `nightly` in ",
            ),
            (
                &unshared,
                &["--benchmark", "nightly"],
                "so hotpath.rs refuses its reports. Add one of them.",
            ),
            (&outside, &[], "is not inside a git repository"),
            (
                &unshared,
                &["--benchmark", "a/b"],
                "invalid --benchmark `a/b`",
            ),
        ];
        for (checkout, args, expected) in cases {
            let output = checkout.validate(&server, None, args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert_eq!(stdout(&output), "", "{args:?}");
            let error = client_error(&output);
            assert!(error.contains(expected), "{args:?}: {error}");
        }
        mock.assert();
    }

    #[test]
    fn validate_policy_stops_on_any_error_but_a_refused_document() {
        let internal = error_body(ApiErrorCode::Internal, "Something went wrong.");
        // A 422 that is not about the document is not a result either.
        let other_422 = error_body(ApiErrorCode::BadRequest, "The body is not JSON.");
        for (status, body) in [(500, internal), (422, other_422)] {
            let mut server = Server::new();
            let mock = server
                .mock("POST", VALIDATE_PATH)
                .with_status(status)
                .with_header("content-type", "application/json")
                .with_body(&body)
                .expect(1)
                .create();
            let checkout = Checkout::new(&format!("stops_{status}"));
            checkout.write("hotpath/policy.toml", GOOD_POLICY.as_bytes());

            let output = checkout.validate(&server, None, &[]);
            assert_eq!(output.status.code(), Some(1), "{body}");
            assert_eq!(stdout(&output), "", "{body}");
            assert_eq!(json(&stderr(&output)), json(&body));
            mock.assert();
        }
    }

    #[test]
    fn validate_policy_usage_errors_are_exit_2() {
        let mut server = Server::new();
        let mock = mock_no_validation(&mut server);
        let checkout = Checkout::new("usage");
        checkout.write("hotpath/policy.toml", GOOD_POLICY.as_bytes());

        let cases: [&[&str]; 3] = [
            &["--benchmark", "ci", "--file", "hotpath/policy.toml"],
            &["--repo", "hotpath-rs/hotpath"],
            &["--benchmark"],
        ];
        for args in cases {
            let output = checkout.validate(&server, None, args);
            assert_eq!(
                output.status.code(),
                Some(2),
                "{args:?}: {}",
                stderr(&output)
            );
            assert_eq!(stdout(&output), "", "{args:?}");
        }
        mock.assert();
    }

    const DEFAULT_POLICY_PATH: &str = "/api/v1/policy/default";
    /// Comments and a non-ASCII character: the file is written byte for byte.
    const DEFAULT_SOURCE: &str =
        "# hotpath policy \u{2192} starting point\n[functions.timing]\njudged = true\n";

    /// The route is public: a request carrying any `Authorization` header
    /// does not match.
    fn mock_default_policy(server: &mut ServerGuard, hits: usize) -> mockito::Mock {
        server
            .mock("GET", DEFAULT_POLICY_PATH)
            .match_header("authorization", Matcher::Missing)
            .match_header("user-agent", Matcher::Regex("^hotpath-cli/[0-9]".into()))
            .with_status(200)
            .with_header("content-type", "application/json; charset=utf-8")
            .with_body(serde_json::json!({ "source": DEFAULT_SOURCE }).to_string())
            .expect(hits)
            .create()
    }

    #[test]
    fn init_writes_the_default_policy_and_never_replaces_it_unasked() {
        let mut server = Server::new();
        let mock = mock_default_policy(&mut server, 4);
        let checkout = Checkout::new("init");

        // No token needed.
        let output = checkout.init(&server, None, &[]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stderr(&output), "");
        assert_eq!(
            json(&stdout(&output)),
            json(r#"{"path":"hotpath/policy.toml","replaced":false}"#)
        );
        assert_eq!(
            checkout.read("hotpath/policy.toml").as_deref(),
            Some(DEFAULT_SOURCE)
        );

        // The file is the repository's now: refused without --force.
        checkout.write("hotpath/policy.toml", GOOD_POLICY.as_bytes());
        let output = checkout.init(&server, None, &[]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        assert!(
            client_error(&output).starts_with("`hotpath/policy.toml` already exists."),
            "{}",
            stderr(&output)
        );
        assert_eq!(
            checkout.read("hotpath/policy.toml").as_deref(),
            Some(GOOD_POLICY)
        );

        let output = checkout.init(&server, Some(TOKEN), &["--force", "--pretty"]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(
            json(&stdout(&output)),
            json(r#"{"path":"hotpath/policy.toml","replaced":true}"#)
        );
        assert_eq!(
            checkout.read("hotpath/policy.toml").as_deref(),
            Some(DEFAULT_SOURCE)
        );

        // A benchmark's own file sits next to the shared one.
        let output = checkout.init(&server, Some(TOKEN), &["--benchmark", "nightly"]);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(
            json(&stdout(&output)),
            json(r#"{"path":"hotpath/nightly-policy.toml","replaced":false}"#)
        );
        assert_eq!(
            checkout.read("hotpath/nightly-policy.toml").as_deref(),
            Some(DEFAULT_SOURCE)
        );
        // --output elsewhere takes the JSON result; the policy is still written.
        let output = checkout.init(
            &server,
            Some(TOKEN),
            &["--benchmark", "ci", "--output", "../result.json"],
        );
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        assert_eq!(stdout(&output), "");
        assert_eq!(
            json(&checkout.read("result.json").unwrap()),
            json(r#"{"path":"hotpath/ci-policy.toml","replaced":false}"#)
        );
        assert_eq!(
            checkout.read("hotpath/ci-policy.toml").as_deref(),
            Some(DEFAULT_SOURCE)
        );
        std::fs::remove_file(checkout.root.join("hotpath/ci-policy.toml")).unwrap();

        // No temporary file is left next to them.
        let mut files: Vec<String> = std::fs::read_dir(checkout.root.join("hotpath"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(files, ["nightly-policy.toml", "policy.toml"]);
        mock.assert();
    }

    #[test]
    fn init_refuses_before_any_request() {
        let mut server = Server::new();
        let mock = mock_default_policy(&mut server, 0);
        let outside = Checkout::without_git("init_outside");
        let linked = Checkout::new("init_symlink");
        linked.write("elsewhere.toml", GOOD_POLICY.as_bytes());
        std::fs::create_dir_all(linked.root.join("hotpath")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            linked.root.join("elsewhere.toml"),
            linked.root.join("hotpath/policy.toml"),
        )
        .unwrap();
        let directory = Checkout::new("init_directory");
        std::fs::create_dir_all(directory.root.join("hotpath/policy.toml")).unwrap();
        let empty = Checkout::new("init_unwritten");

        let mut cases: Vec<(&Checkout, &[&str], &str)> = vec![
            (&outside, &[], "is not inside a git repository"),
            // `init` runs from `src/`: two spellings of the file it writes.
            (
                &empty,
                &["--output", "../hotpath/policy.toml"],
                "--output names `hotpath/policy.toml`",
            ),
            (
                &empty,
                &[
                    "--benchmark",
                    "ci",
                    "--output",
                    "./../hotpath/../hotpath/ci-policy.toml",
                ],
                "--output names `hotpath/ci-policy.toml`",
            ),
            (&directory, &["--force"], "exists and is not a file"),
            (&empty, &["--benchmark", "a/b"], "invalid --benchmark `a/b`"),
        ];
        #[cfg(unix)]
        cases.push((&linked, &["--force"], "is a symlink"));
        for (checkout, args, expected) in cases {
            let output = checkout.init(&server, Some(TOKEN), args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert_eq!(stdout(&output), "", "{args:?}");
            let error = client_error(&output);
            assert!(error.contains(expected), "{args:?}: {error}");
        }
        assert_eq!(
            linked.read("elsewhere.toml").as_deref(),
            Some(GOOD_POLICY),
            "the symlink target was written"
        );

        assert!(!empty.root.join("hotpath").exists());
        mock.assert();
    }

    #[test]
    fn init_passes_a_server_error_through_and_writes_nothing() {
        let mut server = Server::new();
        let body = error_body(ApiErrorCode::Internal, "Something went wrong.");
        let mock = server
            .mock("GET", DEFAULT_POLICY_PATH)
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(&body)
            .create();
        let checkout = Checkout::new("init_error");

        let output = checkout.init(&server, Some(TOKEN), &[]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stdout(&output), "");
        assert_eq!(stderr(&output).trim(), body);
        assert!(!checkout.root.join("hotpath").exists());
        mock.assert();
    }
}
