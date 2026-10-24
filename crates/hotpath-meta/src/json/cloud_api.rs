//! The client contract of the hotpath.rs API: the types a client branches on,
//! shared by the `hotpath-cloud` upload client, the `hotpath cloud` CLI and
//! hotpath-backend, which serializes these types.
//!
//! Bodies a client only prints are owned by hotpath-backend and passed
//! through as received. A field moves here only when a client starts reading
//! it.

use std::fmt;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// Base URL of the hotpath.rs API when nothing overrides it.
pub const DEFAULT_BASE_URL: &str = "https://hotpath.rs";

/// Base URL every client talks to, the upload included. `HOTPATH_META_API_URL`
/// overrides the default for staging or self-hosted backends.
pub static API_URL: LazyLock<String> =
    LazyLock::new(|| normalize_base_url(std::env::var("HOTPATH_META_API_URL").ok()));

/// Turns the raw value of `HOTPATH_META_API_URL` into a base URL: trimmed, without
/// trailing slashes, `DEFAULT_BASE_URL` when unset or blank.
pub fn normalize_base_url(raw: Option<String>) -> String {
    raw.map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// A benchmark name that breaks the rule every party enforces: 1-64 chars of
/// `[A-Za-z0-9._-]`, not `.` or `..`. Displays as the rule itself, so each
/// caller prefixes where the name came from (`HOTPATH_META_BENCHMARK`,
/// `--benchmark`). The server's copy (hotpath-backend
/// `models::benchmark::validate_name`) must stay byte for byte identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidBenchmarkName;

impl fmt::Display for InvalidBenchmarkName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("use 1-64 chars from [A-Za-z0-9._-], not \".\" or \"..\"")
    }
}

/// Checks a benchmark name before it names an upload series or goes into a
/// request path; a valid name needs no percent-encoding.
pub fn validate_benchmark_name(name: &str) -> Result<(), InvalidBenchmarkName> {
    let valid_chars = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if name.is_empty() || name.len() > 64 || !valid_chars || name == "." || name == ".." {
        return Err(InvalidBenchmarkName);
    }
    Ok(())
}

/// Why a request was refused, as clients branch on it. Never on the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    /// No `Authorization: Bearer` credential at all (401).
    MissingToken,
    /// Unknown, expired or revoked token: one code, so a probe learns nothing (401).
    InvalidToken,
    /// The user's GitHub authorization lapsed; log in at hotpath.rs once (401).
    GithubAuthorizationExpired,
    /// Malformed query or path value, or an oversized upload body (400, 413).
    BadRequest,
    /// The token's user may not act on this resource, for instance an upload
    /// to a repository the GitHub App is not installed on (403).
    Forbidden,
    /// Unknown path or resource, including anything the caller may not see (404).
    NotFound,
    MethodNotAllowed,
    /// 429; the `Retry-After` header says how many seconds to wait.
    RateLimited,
    /// A policy was refused (422), sent for validation or carried by an
    /// upload; the body is a `PolicyRejected` with every problem found.
    InvalidPolicy,
    /// An upload carried no policy (422): every report must carry the
    /// repository's policy file. A plain `ApiError`, there is no document to
    /// list problems for.
    PolicyRequired,
    Internal,
    /// A code this client does not know; printed like any other error. Also
    /// the default, so a body without `code` still parses.
    #[default]
    #[serde(other)]
    Unknown,
}

/// Body of every non-2xx answer of `/api/v1`. The request id is the
/// `x-request-id` response header, not part of the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    /// What was wrong and what to do about it.
    pub error: String,
    #[serde(default)]
    pub code: ApiErrorCode,
}

/// What happened to the pull request comment. `url`: posted or updated;
/// `error`: failed; `skipped`: the benchmark chose not to comment; none:
/// nothing to post (e.g. a push upload).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// Body of a successful upload (201, or 200 when the server already had the
/// same run and returned the existing report).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadCreated {
    pub id: String,
    pub repository: String,
    pub benchmark: String,
    /// Report the upload is compared against (pull request uploads only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    /// Defaulted so a server that does not send it yet still parses.
    #[serde(default)]
    pub comment: CommentOutcome,
    /// Judgement under the policy at `policy_path`. Required: a body without
    /// a verdict must not read as "no regression".
    pub verdict: Verdict,
    /// Path of the policy file the report was judged under.
    pub policy_path: String,
    /// That file on GitHub, pinned to the measured commit.
    pub policy_url: String,
    /// The dashboard's page for the report against its baseline
    /// (`.../benchmarks/{benchmark}/reports/{id}/diff`).
    pub dashboard_url: String,
}

/// Largest policy document a report may carry and the server stores, in
/// bytes of UTF-8.
pub const POLICY_MAX_BYTES: usize = 65536;

/// Largest report the server accepts, in bytes of the decompressed JSON
/// body. Clients check it before sending and never truncate a report to fit.
pub const MAX_REPORT_BYTES: usize = 5 * 1024 * 1024;

/// Why a serialized report of `len` bytes cannot be uploaded, or `Ok` when
/// it fits [`MAX_REPORT_BYTES`].
pub fn check_report_size(len: usize) -> Result<(), String> {
    if len <= MAX_REPORT_BYTES {
        return Ok(());
    }
    Err(format!(
        "the report is {len} bytes, over the {} MiB ({MAX_REPORT_BYTES} bytes) limit of \
         hotpath.rs; measure fewer functions or split the benchmark",
        MAX_REPORT_BYTES / (1024 * 1024)
    ))
}

/// Body of `POST /api/v1/policy/validate`. The answer is a 200 when the
/// document is a valid policy, or `PolicyRejected` (422).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyValidation {
    /// The whole TOML document, as written. At most `POLICY_MAX_BYTES`; blank
    /// is refused.
    pub source: String,
}

/// Body of `GET /api/v1/policy/default` (200): a starting policy file listing
/// every metric family with the release's key defaults. A key a policy table
/// omits takes its value from here; an omitted family table is not judged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultPolicy {
    /// The TOML document as written, comments included: they document each
    /// key.
    pub source: String,
}

/// One thing wrong with a submitted policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyProblem {
    /// 1-based line of the submitted text the problem points at; `None` when
    /// it concerns the document as a whole (size, blank) or no line can be
    /// named.
    pub line: Option<u32>,
    /// What is wrong and, where the server can say, what is allowed
    /// (`functions.alloc.min_percent_change must be between 0 and 1000, got 5000`).
    pub message: String,
}

/// 422 body from `POST /api/v1/policy/validate` or an upload whose policy is
/// refused: an `ApiError` (`code` = `invalid_policy`) plus every problem
/// found, so a client that only knows `ApiError` still parses it.
///
/// A TOML syntax error stops parsing and is the only problem reported;
/// otherwise every structural and range problem is reported together. Never
/// assume a single item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRejected {
    /// One sentence summing up (`The policy has 3 problems.`).
    pub error: String,
    /// `InvalidPolicy`.
    pub code: ApiErrorCode,
    /// Never empty. Ordered by line, whole-document problems first.
    pub problems: Vec<PolicyProblem>,
}

/// The answer for a report: the judged families of its diff against the
/// baseline (when there is one) and the policy's budgets, together. The
/// top-level `verdict` of the diff body (`hotpath cloud diff` exits 0 for
/// `judged && !regressed`, 1 for everything else) and of `UploadCreated`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    /// Whether anything was judged: the diff was compared, or the policy has
    /// budget rules and head could be read. `false` for a head that does not
    /// parse, and for a report with no comparable baseline under a policy
    /// without budgets.
    pub judged: bool,
    /// A judged family regressed or a budget is broken:
    /// `regressions > 0 || budgets_broken > 0`.
    pub regressed: bool,
    /// Regressions of judged families; 0 when nothing was compared.
    pub regressions: u64,
    /// Improvements of judged families; 0 when nothing was compared.
    pub improvements: u64,
    /// Broken budget checks; 0 when head was not read.
    pub budgets_broken: u64,
}

#[cfg(test)]
mod tests {
    #[test]
    fn report_size_limit_is_inclusive() {
        use crate::json::cloud_api::{check_report_size, MAX_REPORT_BYTES};
        assert!(check_report_size(MAX_REPORT_BYTES).is_ok());
        let err = check_report_size(MAX_REPORT_BYTES + 1).unwrap_err();
        assert!(err.contains("5242881 bytes"), "{err}");
        assert!(err.contains("split the benchmark"), "{err}");
    }

    use crate::json::cloud_api::{
        normalize_base_url, validate_benchmark_name, ApiError, ApiErrorCode, CommentOutcome,
        PolicyProblem, PolicyRejected, PolicyValidation, UploadCreated, DEFAULT_BASE_URL,
    };

    #[test]
    fn validate_benchmark_name_rule() {
        for ok in [
            "default",
            "ci",
            "timing-linux",
            "api_latency",
            "v0.25",
            "timing.linux",
        ] {
            assert!(
                validate_benchmark_name(ok).is_ok(),
                "{ok:?} should be valid"
            );
        }
        for bad in ["a/b", "a b", "..", ".", "x?y", "ünïcode", ""] {
            assert!(
                validate_benchmark_name(bad).is_err(),
                "{bad:?} should be invalid"
            );
        }
        assert!(validate_benchmark_name(&"a".repeat(64)).is_ok());
        assert!(validate_benchmark_name(&"a".repeat(65)).is_err());

        let err = validate_benchmark_name("a/b").unwrap_err().to_string();
        assert!(
            err.contains("[A-Za-z0-9._-]"),
            "message names the rule: {err}"
        );
    }

    #[test]
    fn normalize_base_url_rules() {
        assert_eq!(normalize_base_url(None), DEFAULT_BASE_URL);
        assert_eq!(normalize_base_url(Some("   ".into())), DEFAULT_BASE_URL);
        assert_eq!(
            normalize_base_url(Some(" http://localhost:3000/// ".into())),
            "http://localhost:3000"
        );
        assert_eq!(
            normalize_base_url(Some("https://staging.hotpath.rs".into())),
            "https://staging.hotpath.rs"
        );
    }

    #[test]
    fn api_error_parses_known_unknown_and_missing_codes() {
        let err: ApiError =
            serde_json::from_str(r#"{"error":"nope","code":"invalid_token","later":1}"#).unwrap();
        assert_eq!(err.error, "nope");
        assert_eq!(err.code, ApiErrorCode::InvalidToken);

        let newer: ApiError =
            serde_json::from_str(r#"{"error":"nope","code":"quota_exceeded"}"#).unwrap();
        assert_eq!(newer.code, ApiErrorCode::Unknown);

        let bare: ApiError = serde_json::from_str(r#"{"error":"boom"}"#).unwrap();
        assert_eq!(bare.code, ApiErrorCode::Unknown);

        assert_eq!(
            serde_json::to_string(&ApiError {
                error: "Benchmark ci not found.".into(),
                code: ApiErrorCode::NotFound,
            })
            .unwrap(),
            r#"{"error":"Benchmark ci not found.","code":"not_found"}"#
        );
    }

    const UPLOAD_DASHBOARD_URL: &str =
        "https://hotpath.rs/app/repos/a/b/benchmarks/meta/reports/r1/diff";
    const POLICY_URL: &str =
        "https://github.com/a/b/blob/3f1c000000000000000000000000000000000000/hotpath/policy.toml";

    #[test]
    fn upload_created_parses_with_and_without_comment() {
        let full: UploadCreated = serde_json::from_str(&format!(
            r#"{{"id":"r1","repository":"a/b","benchmark":"meta","baseline":"r0",
                "comment":{{"url":"https://github.com/c/1","error":"the report does not parse"}},
                "verdict":{{"judged":true,"regressed":false,"regressions":0,"improvements":2,"budgets_broken":0}},
                "policy_path":"hotpath/policy.toml","policy_url":"{POLICY_URL}",
                "dashboard_url":"{UPLOAD_DASHBOARD_URL}","later":true}}"#
        ))
        .unwrap();
        assert_eq!(full.baseline.as_deref(), Some("r0"));
        assert_eq!(full.comment.url.as_deref(), Some("https://github.com/c/1"));
        assert_eq!(
            full.comment.error.as_deref(),
            Some("the report does not parse")
        );
        assert_eq!(full.verdict.improvements, 2);

        let bare: UploadCreated = serde_json::from_str(&format!(
            r#"{{"id":"r1","repository":"a/b","benchmark":"meta",
                "verdict":{{"judged":false,"regressed":false,"regressions":0,"improvements":0,"budgets_broken":0}},
                "policy_path":"hotpath/policy.toml","policy_url":"{POLICY_URL}",
                "dashboard_url":"{UPLOAD_DASHBOARD_URL}"}}"#
        ))
        .unwrap();
        assert_eq!(bare.baseline, None);
        assert_eq!(bare.comment, CommentOutcome::default());
    }

    #[test]
    fn upload_created_names_the_policy_that_judged() {
        let body = format!(
            r#"{{"id":"r1","repository":"a/b","benchmark":"meta","comment":{{}},"verdict":{{"judged":true,"regressed":false,"regressions":0,"improvements":0,"budgets_broken":0}},"policy_path":"hotpath/policy.toml","policy_url":"{POLICY_URL}","dashboard_url":"{UPLOAD_DASHBOARD_URL}"}}"#
        );
        let created: UploadCreated = serde_json::from_str(&body).unwrap();
        assert_eq!(created.policy_path, "hotpath/policy.toml");
        assert_eq!(created.policy_url, POLICY_URL);
        assert_eq!(serde_json::to_string(&created).unwrap(), body);
    }

    #[test]
    fn upload_created_requires_the_verdict() {
        let no_verdict = format!(
            r#"{{"id":"r1","repository":"a/b","benchmark":"meta","dashboard_url":"{UPLOAD_DASHBOARD_URL}"}}"#
        );
        assert!(serde_json::from_str::<UploadCreated>(&no_verdict).is_err());
        let no_url = r#"{"id":"r1","repository":"a/b","benchmark":"meta","verdict":{"judged":true,"regressed":false,"regressions":0,"improvements":0,"budgets_broken":0}}"#;
        assert!(serde_json::from_str::<UploadCreated>(no_url).is_err());
    }

    #[test]
    fn policy_validation_round_trips() {
        let body = r#"{"source":"[functions]\n"}"#;
        let request: PolicyValidation = serde_json::from_str(body).unwrap();
        assert_eq!(
            request,
            PolicyValidation {
                source: "[functions]\n".into(),
            }
        );
        assert_eq!(serde_json::to_string(&request).unwrap(), body);
    }

    #[test]
    fn policy_rejected_round_trips_and_parses_as_api_error() {
        let body = r#"{"error":"The policy has 3 problems.","code":"invalid_policy","problems":[{"line":null,"message":"the policy is larger than 65536 bytes"},{"line":3,"message":"unknown key `functions.timing.min_percent`"},{"line":7,"message":"functions.alloc.min_percent_change must be between 0 and 1000, got 5000"}]}"#;
        let rejected: PolicyRejected = serde_json::from_str(body).unwrap();
        assert_eq!(
            rejected,
            PolicyRejected {
                error: "The policy has 3 problems.".into(),
                code: ApiErrorCode::InvalidPolicy,
                problems: vec![
                    PolicyProblem {
                        line: None,
                        message: "the policy is larger than 65536 bytes".into(),
                    },
                    PolicyProblem {
                        line: Some(3),
                        message: "unknown key `functions.timing.min_percent`".into(),
                    },
                    PolicyProblem {
                        line: Some(7),
                        message:
                            "functions.alloc.min_percent_change must be between 0 and 1000, got 5000"
                                .into(),
                    },
                ],
            }
        );
        assert_eq!(serde_json::to_string(&rejected).unwrap(), body);

        let plain: ApiError = serde_json::from_str(body).unwrap();
        assert_eq!(plain.code, ApiErrorCode::InvalidPolicy);
        assert_eq!(plain.error, "The policy has 3 problems.");
    }
}
