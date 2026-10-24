//! Uploads the JSON report to hotpath.rs from GitHub Actions, authenticated
//! with the job's OIDC token. Configured by `HOTPATH_UPLOAD` (`UploadMode`),
//! `HOTPATH_BENCHMARK` (default `default`; an invalid name skips the upload),
//! `HOTPATH_API_URL` (default `https://hotpath.rs`) and
//! `HOTPATH_UPLOAD_RESPONSE_PATH` (response body written for custom rules).
//!
//! Runs synchronously from the guard's `Drop`, after the runtime may already
//! be gone, so it never spawns tasks.
//!
//! Every run ends in one `Outcome`, rendered into one line at one level: to
//! stderr, and under GitHub Actions also as a workflow command plus a
//! `GITHUB_STEP_SUMMARY` block. The server owns the message text; the client
//! branches only on `verdict.regressed`, which is a warning and never fails
//! the job: the server fails the pull request's check run instead when the
//! policy asks for it. A failed upload exits 1 under `fail-on-error`, once
//! the local report is written; skips never fail. A report without a policy
//! fails before a token is minted.
//!
//! No retries yet: a retry is only safe once the server insert is idempotent
//! per run, otherwise a timed-out upload that was in fact stored would be
//! duplicated.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use serde::Deserialize;

use crate::json::cloud_api::{
    check_report_size, validate_benchmark_name, ApiError, ApiErrorCode, PolicyProblem,
    PolicyRejected, UploadCreated, Verdict, API_URL,
};
use crate::json::policy_file::policy_files_hint;
use crate::json::JsonReport;

const AUDIENCE: &str = "hotpath.rs";
const MINT_TIMEOUT: Duration = Duration::from_secs(10);
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest raw (unparseable) response body quoted in a message.
const MAX_QUOTED_BODY: usize = 2000;
/// Most problems of a refused policy listed in a message; the step summary
/// has the whole body.
const MAX_LISTED_PROBLEMS: usize = 20;

/// `HOTPATH_UPLOAD`: whether the report is uploaded and whether a failed
/// upload fails the job. A regression never does: the pull request's
/// `hotpath / <benchmark>` check run fails when the policy sets
/// `pr_check = "fail"` and the finding comes from a `fail_check` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum UploadMode {
    /// Unset, empty, `0` or `false`.
    Off,
    /// `enabled`, `1` or `true`: failures and regressions are warnings, so an
    /// adopter's job never goes red because hotpath.rs is down.
    Enabled,
    /// `fail-on-error`: a failed upload is an `::error::` and exits 1.
    FailOnError,
}

impl UploadMode {
    fn parse(v: &str) -> Option<Self> {
        match v.trim().to_ascii_lowercase().as_str() {
            "" | "0" | "false" => Some(UploadMode::Off),
            "enabled" | "1" | "true" => Some(UploadMode::Enabled),
            "fail-on-error" => Some(UploadMode::FailOnError),
            _ => None,
        }
    }
}

/// An unknown value uploads without failing the job: a typo must not
/// silently turn the upload off.
pub(crate) static UPLOAD: LazyLock<UploadMode> = LazyLock::new(|| {
    let value = std::env::var("HOTPATH_UPLOAD").unwrap_or_default();
    UploadMode::parse(&value).unwrap_or_else(|| {
        eprintln!(
            "hotpath: unknown HOTPATH_UPLOAD {value:?}, uploading as \"enabled\"; \
             expected enabled or fail-on-error"
        );
        UploadMode::Enabled
    })
});

/// `HOTPATH_UPLOAD_RESPONSE_PATH`: file the response body of an upload is
/// written to. A file and not stdout, which belongs to the profiled program.
/// Blank or unset writes nothing.
pub(crate) static RESPONSE_PATH: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    std::env::var_os("HOTPATH_UPLOAD_RESPONSE_PATH")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
});

pub(crate) fn enabled() -> bool {
    *UPLOAD != UploadMode::Off
}

fn truthy_var(name: &str) -> bool {
    std::env::var(name).map(|v| is_truthy(&v)).unwrap_or(false)
}

fn is_truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true")
}

pub(crate) fn benchmark_name() -> Result<String, String> {
    let name = match std::env::var("HOTPATH_BENCHMARK") {
        Ok(s) => s.trim().to_string(),
        Err(std::env::VarError::NotPresent) => String::new(),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("invalid HOTPATH_BENCHMARK: value is not valid UTF-8".to_string())
        }
    };
    if name.is_empty() {
        return Ok("default".to_string());
    }
    // Fails fast before a token is minted and the report serialized.
    validate_benchmark_name(&name)
        .map_err(|rule| format!("invalid HOTPATH_BENCHMARK {name:?}: {rule}"))?;
    Ok(name)
}

/// How one upload attempt ended.
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    /// Nothing was sent and nothing is wrong with the server: not in Actions,
    /// no token, invalid benchmark name, the program panicked. Never fails,
    /// whatever the `HOTPATH_UPLOAD` mode.
    Skipped(String),
    Uploaded {
        /// The part of `body` this client reads.
        created: Box<UploadCreated>,
        /// The response body as received, with every field the server sent,
        /// known to this client or not.
        body: String,
        /// From the `x-request-id` response header.
        request_id: Option<String>,
    },
    Failed {
        /// Built where the failure happened: the server's `error` sentence
        /// for a rejection, the transport error otherwise.
        message: String,
        /// The response body, when there was one, for the step summary.
        body: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Level {
    Notice,
    Warning,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Notice => "notice",
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }
}

/// One line at one level, plus the step summary block.
#[derive(Debug, PartialEq)]
pub(crate) struct Rendered {
    level: Level,
    /// Printed to stderr as `hotpath: <message>` and, in Actions, once more
    /// as `::<level>::hotpath: <message>`.
    message: String,
    /// Markdown appended to `GITHUB_STEP_SUMMARY`.
    summary: String,
}

pub(crate) struct Env {
    /// `GITHUB_ACTIONS` is set: emit workflow commands.
    actions: bool,
    mode: UploadMode,
    /// `GITHUB_STEP_SUMMARY`, when set and non-empty.
    summary: Option<PathBuf>,
}

impl Env {
    fn from_process() -> Self {
        Env {
            actions: truthy_var("GITHUB_ACTIONS"),
            mode: *UPLOAD,
            summary: std::env::var_os("GITHUB_STEP_SUMMARY")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from),
        }
    }
}

/// Returns `true` when the caller must exit 1: a failed upload under
/// `fail-on-error`. The response file is updated first.
pub(crate) fn upload(report: &JsonReport) -> bool {
    let outcome = run(report);
    let env = Env::from_process();
    if let Some(path) = RESPONSE_PATH.as_deref() {
        if let Err(e) = store_response(&outcome, path) {
            eprintln!("hotpath: could not write {}: {e}", path.display());
        }
    }
    let benchmark = benchmark_name().ok();
    let rendered = render(&outcome, &env, benchmark.as_deref());
    let failed = rendered.level == Level::Error;
    emit(rendered, &env);
    failed
}

fn run(report: &JsonReport) -> Outcome {
    if std::thread::panicking() {
        return Outcome::Skipped("the profiled program panicked".to_string());
    }
    let (request_url, request_token) = match (
        std::env::var("ACTIONS_ID_TOKEN_REQUEST_URL"),
        std::env::var("ACTIONS_ID_TOKEN_REQUEST_TOKEN"),
    ) {
        (Ok(url), Ok(token)) if !url.is_empty() && !token.is_empty() => (url, token),
        _ => {
            return Outcome::Skipped(
                "not in GitHub Actions or missing `id-token: write` permission".to_string(),
            )
        }
    };
    let benchmark = match benchmark_name() {
        Ok(name) => name,
        Err(msg) => return Outcome::Skipped(msg),
    };
    // Refused before a token is minted.
    if report.meta.policy.is_none() {
        return Outcome::Failed {
            message: missing_policy_message(Some(&benchmark)),
            body: None,
        };
    }
    // Serialized and size-checked before a token is minted: an oversized
    // report is refused here rather than truncated or sent to a sure 413.
    let body = match serde_json::to_vec(report) {
        Ok(body) => body,
        Err(e) => {
            return Outcome::Failed {
                message: format!("failed to serialize report: {e}"),
                body: None,
            }
        }
    };
    if let Err(message) = check_report_size(body.len()) {
        return Outcome::Failed {
            message,
            body: None,
        };
    }
    let token = match mint_token(&request_url, &request_token) {
        Ok(token) => token,
        Err(message) => {
            return Outcome::Failed {
                message,
                body: None,
            }
        }
    };
    post_report(&API_URL, &token, &benchmark, &body)
}

/// Why a report without `meta.policy` is not sent, and what to add.
pub(crate) fn missing_policy_message(benchmark: Option<&str>) -> String {
    format!(
        "the report carries no policy file and hotpath.rs refuses reports without one. Add {} \
         to the repository (`hotpath cloud init` writes the default policy) and check it \
         with `hotpath cloud validate-policy`.",
        policy_files_hint(benchmark)
    )
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into()
}

#[derive(Deserialize)]
struct TokenResponse {
    value: String,
}

pub(crate) fn mint_token(request_url: &str, request_token: &str) -> Result<String, String> {
    let separator = if request_url.contains('?') { '&' } else { '?' };
    let url = format!("{request_url}{separator}audience={AUDIENCE}");
    let mut resp = agent(MINT_TIMEOUT)
        .get(&url)
        .header("Authorization", &format!("bearer {request_token}"))
        .call()
        .map_err(|e| format!("OIDC token request failed: {e}"))?;
    let status = resp.status().as_u16();
    if status != 200 {
        let body = read_body(&mut resp);
        return Err(format!("OIDC token request returned HTTP {status}: {body}"));
    }
    let token: TokenResponse = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("invalid OIDC token response: {e}"))?;
    Ok(token.value)
}

/// Sends the JSON `body` gzipped under `Content-Encoding: gzip`; the server
/// caps and meters the decompressed size.
pub(crate) fn post_report(base_url: &str, token: &str, benchmark: &str, body: &[u8]) -> Outcome {
    let url = format!(
        "{base_url}/api/v1/reports?benchmark={}",
        url_encode(benchmark)
    );
    let body = match gzip(body) {
        Ok(body) => body,
        Err(e) => {
            return Outcome::Failed {
                message: format!("failed to compress report: {e}"),
                body: None,
            }
        }
    };
    let mut resp = match agent(UPLOAD_TIMEOUT)
        .post(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("Content-Encoding", "gzip")
        .header("X-Hotpath-Version", env!("CARGO_PKG_VERSION"))
        .send(&body[..])
    {
        Ok(resp) => resp,
        Err(e) => {
            return Outcome::Failed {
                message: format!("request to {base_url} failed: {e}"),
                body: None,
            }
        }
    };
    let status = resp.status().as_u16();
    let request_id = resp
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = read_body(&mut resp);
    interpret(status, request_id, body)
}

fn gzip(body: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(body)?;
    encoder.finish()
}

/// A 201 (or the 200 of an already stored run) parses as `UploadCreated`,
/// anything else tries `ApiError` and falls back to quoting the raw body. A
/// refused policy (`PolicyRejected`) lists its problems after the sentence,
/// since the sentence alone only counts them.
pub(crate) fn interpret(status: u16, request_id: Option<String>, body: String) -> Outcome {
    let request = |id: Option<&str>| id.map(|id| format!(", request {id}")).unwrap_or_default();
    if matches!(status, 200 | 201) {
        return match serde_json::from_str::<UploadCreated>(&body) {
            Ok(created) => Outcome::Uploaded {
                created: Box::new(created),
                body,
                request_id,
            },
            Err(_) => Outcome::Failed {
                message: format!(
                    "HTTP {status} but the response could not be read, the report was probably stored{}: {}",
                    request(request_id.as_deref()),
                    quote_body(&body)
                ),
                body: Some(body),
            },
        };
    }
    let message = match serde_json::from_str::<ApiError>(&body) {
        Ok(error) => format!(
            "{}{} (HTTP {status}{})",
            error.error,
            policy_problems(&error, &body),
            request(request_id.as_deref())
        ),
        Err(_) => format!(
            "HTTP {status}{}: {}",
            request(request_id.as_deref()),
            quote_body(&body)
        ),
    };
    Outcome::Failed {
        message,
        body: Some(body),
    }
}

/// The problems of a refused policy as ` <problem>; <problem>`, empty for any
/// other rejection.
fn policy_problems(error: &ApiError, body: &str) -> String {
    if error.code != ApiErrorCode::InvalidPolicy {
        return String::new();
    }
    let Ok(rejected) = serde_json::from_str::<PolicyRejected>(body) else {
        return String::new();
    };
    let describe = |problem: &PolicyProblem| match problem.line {
        Some(line) => format!("line {line}: {}", problem.message),
        None => problem.message.clone(),
    };
    let mut listed: Vec<String> = rejected
        .problems
        .iter()
        .take(MAX_LISTED_PROBLEMS)
        .map(describe)
        .collect();
    if let Some(more) = rejected.problems.len().checked_sub(MAX_LISTED_PROBLEMS) {
        if more > 0 {
            listed.push(format!("and {more} more"));
        }
    }
    if listed.is_empty() {
        return String::new();
    }
    format!(" {}", listed.join("; "))
}

/// Writes the body of an upload to `path` byte for byte as received, so the
/// file carries the fields this client does not know yet too. A
/// skipped or failed upload has no verdict: a file already at `path` is
/// removed, so a stale verdict from an earlier step is never read as this
/// run's.
pub(crate) fn store_response(outcome: &Outcome, path: &Path) -> std::io::Result<()> {
    match outcome {
        Outcome::Uploaded { body, .. } => std::fs::write(path, body),
        Outcome::Skipped(_) | Outcome::Failed { .. } => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// The verdict in words, built from its numbers and never from server text.
fn verdict_summary(verdict: &Verdict) -> String {
    let count = |n: u64, one: &str, many: &str| match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {many}")),
    };
    let parts: Vec<String> = [
        count(verdict.regressions, "regression", "regressions"),
        count(verdict.budgets_broken, "budget broken", "budgets broken"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !parts.is_empty() {
        return parts.join(", ");
    }
    if verdict.judged {
        "no regressions".to_string()
    } else {
        "not judged".to_string()
    }
}

/// A received body indented for the step summary; as received when it does
/// not re-parse. Keys come out sorted (no `serde_json/preserve_order` in the
/// library); `HOTPATH_UPLOAD_RESPONSE_PATH` keeps the server's bytes.
fn pretty_body(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .and_then(|value| serde_json::to_string_pretty(&value))
        .unwrap_or_else(|_| body.trim().to_string())
}

/// One message at one level.
pub(crate) fn render(outcome: &Outcome, env: &Env, benchmark: Option<&str>) -> Rendered {
    let (level, message, links, body) = match outcome {
        Outcome::Skipped(reason) => (
            Level::Notice,
            format!("upload skipped: {reason}"),
            Vec::new(),
            None,
        ),
        Outcome::Uploaded {
            created,
            body,
            request_id,
        } => {
            let mut message = format!(
                "uploaded report {} (repository {}, benchmark {}, baseline {}{})",
                created.id,
                created.repository,
                created.benchmark,
                created.baseline.as_deref().unwrap_or("none"),
                request_id
                    .as_deref()
                    .map(|id| format!(", request {id}"))
                    .unwrap_or_default(),
            );
            let verdict = &created.verdict;
            message.push_str(&format!("; verdict: {}", verdict_summary(verdict)));
            if let Some(error) = &created.comment.error {
                message.push_str(&format!("; comment failed: {error}"));
            } else if let Some(reason) = &created.comment.skipped {
                message.push_str(&format!("; comment skipped: {reason}"));
            }
            message.push_str(&format!("; diff: {}", created.dashboard_url));
            let level = if verdict.regressed || created.comment.error.is_some() {
                Level::Warning
            } else {
                Level::Notice
            };
            let links = vec![
                format!("[Open the report on hotpath.rs]({})", created.dashboard_url),
                format!("[Policy: {}]({})", created.policy_path, created.policy_url),
            ];
            (level, message, links, Some(pretty_body(body)))
        }
        Outcome::Failed { message, body } => {
            let level = if env.mode >= UploadMode::FailOnError {
                Level::Error
            } else {
                Level::Warning
            };
            let body = body.as_deref().filter(|b| !b.trim().is_empty());
            (
                level,
                format!("upload failed: {message}"),
                Vec::new(),
                body.map(str::to_string),
            )
        }
    };

    let mut summary = format!(
        "## {}\n\n{}: hotpath: {message}\n",
        summary_heading(benchmark),
        level.as_str()
    );
    for link in links {
        summary.push_str(&format!("\n{link}\n"));
    }
    if let Some(body) = body {
        summary.push_str(&format!("\n```\n{body}\n```\n"));
    }

    Rendered {
        level,
        message,
        summary,
    }
}

fn summary_heading(benchmark: Option<&str>) -> String {
    match benchmark {
        Some(name) => format!("hotpath.rs {name} benchmark"),
        None => "hotpath.rs upload".to_string(),
    }
}

/// A failure outside the upload itself (a requested merge), rendered like a
/// strict upload failure: one error line, plus the annotation and step
/// summary under Actions. No upload mode softens it.
pub(crate) fn emit_error(message: &str) {
    let benchmark = benchmark_name().ok();
    let summary = format!(
        "## {}\n\nerror: hotpath: {message}\n",
        summary_heading(benchmark.as_deref())
    );
    emit(
        Rendered {
            level: Level::Error,
            message: message.to_string(),
            summary,
        },
        &Env::from_process(),
    );
}

fn quote_body(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return "empty body".to_string();
    }
    match body.char_indices().nth(MAX_QUOTED_BODY) {
        Some((cut, _)) => format!("{}... ({} bytes)", &body[..cut], body.len()),
        None => body.to_string(),
    }
}

/// Escapes the message part of a workflow command (`::error::<message>`).
/// Unescaped, a newline ends the command and the rest of the text is lost.
pub(crate) fn escape_annotation(message: &str) -> String {
    message
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Workflow commands are read from stdout, the human line goes to stderr.
fn emit(rendered: Rendered, env: &Env) {
    eprintln!("hotpath: {}", rendered.message);
    if env.actions {
        println!(
            "::{}::hotpath: {}",
            rendered.level.as_str(),
            escape_annotation(&rendered.message)
        );
    }
    if let Some(path) = &env.summary {
        let appended = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .and_then(|mut f| f.write_all(rendered.summary.as_bytes()));
        if let Err(e) = appended {
            eprintln!("hotpath: could not write {}: {e}", path.display());
        }
    }
}

fn read_body(resp: &mut ureq::http::Response<ureq::Body>) -> String {
    resp.body_mut().read_to_string().unwrap_or_default()
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::json::cloud_api::{CommentOutcome, UploadCreated, Verdict};
    use crate::lib_on::cloud::{
        benchmark_name, escape_annotation, interpret, is_truthy, render, store_response,
        url_encode, Env, Level, Outcome, UploadMode,
    };

    const DASHBOARD_URL: &str =
        "https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/meta/reports/r1/diff";
    const POLICY_URL: &str = "https://github.com/hotpath-rs/hotpath/blob/3f1c000000000000000000000000000000000000/hotpath/policy.toml";
    const VERDICT_FIELDS: &str = r#""verdict":{"judged":true,"regressed":false,"regressions":0,"improvements":0,"budgets_broken":0},"policy_path":"hotpath/policy.toml","policy_url":"https://github.com/a/b/blob/3f1c000000000000000000000000000000000000/hotpath/policy.toml","dashboard_url":"https://hotpath.rs/d""#;

    fn env(actions: bool, mode: UploadMode) -> Env {
        Env {
            actions,
            mode,
            summary: None,
        }
    }

    fn verdict(regressions: u64, budgets_broken: u64) -> Verdict {
        Verdict {
            judged: true,
            regressed: regressions > 0 || budgets_broken > 0,
            regressions,
            improvements: 0,
            budgets_broken,
        }
    }

    fn uploaded(verdict: Verdict) -> Outcome {
        uploaded_as(
            UploadCreated {
                verdict,
                ..created()
            },
            None,
        )
    }

    /// `created` as the server would send it.
    fn uploaded_as(created: UploadCreated, request_id: Option<&str>) -> Outcome {
        Outcome::Uploaded {
            body: serde_json::to_string(&created).unwrap(),
            created: Box::new(created),
            request_id: request_id.map(str::to_string),
        }
    }

    fn scratch_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hotpath-cloud-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn created() -> UploadCreated {
        UploadCreated {
            id: "r1".into(),
            repository: "hotpath-rs/hotpath".into(),
            benchmark: "meta".into(),
            baseline: Some("r0".into()),
            comment: CommentOutcome::default(),
            verdict: verdict(0, 0),
            policy_path: "hotpath/policy.toml".into(),
            policy_url: POLICY_URL.into(),
            dashboard_url: DASHBOARD_URL.into(),
        }
    }

    #[test]
    fn upload_mode_values() {
        assert_eq!(UploadMode::parse(""), Some(UploadMode::Off));
        assert_eq!(UploadMode::parse("0"), Some(UploadMode::Off));
        assert_eq!(UploadMode::parse("false"), Some(UploadMode::Off));
        assert_eq!(UploadMode::parse("1"), Some(UploadMode::Enabled));
        assert_eq!(UploadMode::parse(" TRUE "), Some(UploadMode::Enabled));
        assert_eq!(UploadMode::parse("enabled"), Some(UploadMode::Enabled));
        assert_eq!(
            UploadMode::parse("fail-on-error"),
            Some(UploadMode::FailOnError)
        );
        assert_eq!(UploadMode::parse("strict"), None);
    }

    #[test]
    fn truthy_values() {
        assert!(is_truthy("1"));
        assert!(is_truthy("true"));
        assert!(is_truthy(" TRUE "));
        assert!(!is_truthy("0"));
        assert!(!is_truthy("false"));
        assert!(!is_truthy(""));
    }

    #[test]
    fn url_encode_escapes_reserved_chars() {
        assert_eq!(url_encode("timing-linux_1.0"), "timing-linux_1.0");
        assert_eq!(url_encode("a b/c"), "a%20b%2Fc");
    }

    // All HOTPATH_BENCHMARK cases live in one test so env access stays serialized.
    #[test]
    fn benchmark_name_from_env() {
        let var = "HOTPATH_BENCHMARK";
        std::env::remove_var(var);
        assert_eq!(benchmark_name(), Ok("default".to_string()));
        std::env::set_var(var, "  ");
        assert_eq!(benchmark_name(), Ok("default".to_string()));
        std::env::set_var(var, " ci ");
        assert_eq!(benchmark_name(), Ok("ci".to_string()));
        std::env::set_var(var, "a/b");
        assert_eq!(
            benchmark_name(),
            Err(
                "invalid HOTPATH_BENCHMARK \"a/b\": use 1-64 chars from [A-Za-z0-9._-], not \".\" or \"..\""
                    .to_string()
            )
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            std::env::set_var(var, std::ffi::OsStr::from_bytes(b"\xFF\xFE"));
            let err = benchmark_name().unwrap_err();
            assert!(err.contains("not valid UTF-8"), "{err}");
        }
        std::env::remove_var(var);
    }

    #[test]
    fn interpret_success_bodies() {
        let ok = interpret(
            201,
            Some("abc".into()),
            format!(
                r#"{{"id":"r1","repository":"a/b","benchmark":"meta","comment":{{"url":"https://github.com/c/1"}},{VERDICT_FIELDS}}}"#
            ),
        );
        match ok {
            Outcome::Uploaded {
                created,
                request_id,
                ..
            } => {
                assert_eq!(created.id, "r1");
                assert_eq!(
                    created.comment.url.as_deref(),
                    Some("https://github.com/c/1")
                );
                assert_eq!(request_id.as_deref(), Some("abc"));
            }
            other => panic!("expected Uploaded, got {other:?}"),
        }

        // 200: the server already had this run.
        assert!(matches!(
            interpret(
                200,
                None,
                format!(r#"{{"id":"r1","repository":"a/b","benchmark":"meta",{VERDICT_FIELDS}}}"#)
            ),
            Outcome::Uploaded { .. }
        ));

        let no_verdict = r#"{"id":"r1","repository":"a/b","benchmark":"meta","dashboard_url":"https://hotpath.rs/d"}"#;
        assert_eq!(
            interpret(201, None, no_verdict.into()),
            Outcome::Failed {
                message: format!(
                    "HTTP 201 but the response could not be read, the report was probably stored: {no_verdict}"
                ),
                body: Some(no_verdict.into()),
            }
        );

        // Any other status is a failure even with a success-shaped body.
        assert_eq!(
            interpret(
                202,
                None,
                r#"{"id":"r1","repository":"a/b","benchmark":"meta"}"#.into()
            ),
            Outcome::Failed {
                message: r#"HTTP 202: {"id":"r1","repository":"a/b","benchmark":"meta"}"#.into(),
                body: Some(r#"{"id":"r1","repository":"a/b","benchmark":"meta"}"#.into()),
            }
        );

        assert_eq!(
            interpret(201, Some("abc".into()), r#"{"id":"r1","repo"#.into()),
            Outcome::Failed {
                message: r#"HTTP 201 but the response could not be read, the report was probably stored, request abc: {"id":"r1","repo"#.into(),
                body: Some(r#"{"id":"r1","repo"#.into()),
            }
        );
    }

    #[test]
    fn interpret_error_bodies() {
        let body = r#"{"error":"meta.ci.event is \"pull_request\" but the token was issued to a \"workflow_run\" run. Forward it through hotpath-relay.yml.","code":"bad_request"}"#;
        assert_eq!(
            interpret(400, Some("1bac4db9-15a".into()), body.into()),
            Outcome::Failed {
                message: "meta.ci.event is \"pull_request\" but the token was issued to a \"workflow_run\" run. Forward it through hotpath-relay.yml. (HTTP 400, request 1bac4db9-15a)".into(),
                body: Some(body.into()),
            }
        );

        // A refused policy says what is wrong with it, not only how much.
        let refused = r#"{"error":"The policy hotpath/ci-policy.toml has 2 problems.","code":"invalid_policy","problems":[{"line":null,"message":"the policy sets no column"},{"line":3,"message":"unknown key `functions.timing.min_percent`"}]}"#;
        assert_eq!(
            interpret(422, Some("abc".into()), refused.into()),
            Outcome::Failed {
                message: "The policy hotpath/ci-policy.toml has 2 problems. the policy sets no column; line 3: unknown key `functions.timing.min_percent` (HTTP 422, request abc)".into(),
                body: Some(refused.into()),
            }
        );
        // A long list is cut, and a body without problems is only its sentence.
        let problems: Vec<String> = (1..=25)
            .map(|line| format!(r#"{{"line":{line},"message":"bad"}}"#))
            .collect();
        let many = format!(
            r#"{{"error":"The policy has 25 problems.","code":"invalid_policy","problems":[{}]}}"#,
            problems.join(",")
        );
        let Outcome::Failed { message, .. } = interpret(422, None, many) else {
            panic!()
        };
        assert!(
            message.ends_with("line 20: bad; and 5 more (HTTP 422)"),
            "{message}"
        );
        assert!(!message.contains("line 21"), "{message}");
        assert_eq!(
            interpret(
                422,
                None,
                r#"{"error":"The policy is not valid.","code":"invalid_policy"}"#.into()
            ),
            Outcome::Failed {
                message: "The policy is not valid. (HTTP 422)".into(),
                body: Some(
                    r#"{"error":"The policy is not valid.","code":"invalid_policy"}"#.into()
                ),
            }
        );

        // An unknown code and extra fields are ignored.
        assert_eq!(
            interpret(
                500,
                Some("hdr".into()),
                r#"{"error":"database error","code":"x"}"#.into()
            ),
            Outcome::Failed {
                message: "database error (HTTP 500, request hdr)".into(),
                body: Some(r#"{"error":"database error","code":"x"}"#.into()),
            }
        );

        // Empty 408 from the router's timeout layer: only the header id survives.
        assert_eq!(
            interpret(408, Some("deadbeef".into()), String::new()),
            Outcome::Failed {
                message: "HTTP 408, request deadbeef: empty body".into(),
                body: Some(String::new()),
            }
        );

        // A proxy's HTML, and a body too long to quote whole.
        assert_eq!(
            interpret(502, None, "<html>Bad Gateway</html>".into()),
            Outcome::Failed {
                message: "HTTP 502: <html>Bad Gateway</html>".into(),
                body: Some("<html>Bad Gateway</html>".into()),
            }
        );
        let Outcome::Failed { message, .. } = interpret(502, None, "x".repeat(2500)) else {
            panic!()
        };
        assert!(message.ends_with("... (2500 bytes)"), "{message}");
        assert!(message.len() < 2100);
    }

    #[test]
    fn render_uploaded() {
        // The step summary shows the received body, a field this client does
        // not know included.
        let body = serde_json::to_string(&created()).unwrap();
        let outcome = Outcome::Uploaded {
            created: Box::new(created()),
            body: body.replacen('{', r#"{"new_field":1,"#, 1),
            request_id: Some("abc".into()),
        };
        let r = render(&outcome, &env(true, UploadMode::Enabled), Some("meta"));
        assert_eq!(r.level, Level::Notice);
        assert_eq!(
            r.message,
            format!("uploaded report r1 (repository hotpath-rs/hotpath, benchmark meta, baseline r0, request abc); verdict: no regressions; diff: {DASHBOARD_URL}")
        );
        assert!(r
            .summary
            .starts_with("## hotpath.rs meta benchmark\n\nnotice: hotpath: uploaded report r1"));
        assert!(
            r.summary.contains(&format!(
                "\n[Open the report on hotpath.rs]({DASHBOARD_URL})\n\n[Policy: hotpath/policy.toml]({POLICY_URL})\n\n```\n{{\n  \""
            )),
            "{}",
            r.summary
        );
        assert!(r.summary.contains("\n  \"new_field\": 1"), "{}", r.summary);
        assert!(r.summary.contains("\n  \"id\": \"r1\""), "{}", r.summary);
        assert!(r.summary.ends_with("\n}\n```\n"), "{}", r.summary);

        // Failing on errors changes nothing on success.
        assert_eq!(
            render(&outcome, &env(true, UploadMode::FailOnError), Some("meta")).level,
            Level::Notice
        );

        let no_baseline = uploaded_as(
            UploadCreated {
                baseline: None,
                ..created()
            },
            None,
        );
        assert_eq!(
            render(&no_baseline, &env(false, UploadMode::Enabled), None).message,
            format!("uploaded report r1 (repository hotpath-rs/hotpath, benchmark meta, baseline none); verdict: no regressions; diff: {DASHBOARD_URL}")
        );
    }

    #[test]
    fn render_regressed_levels() {
        let prefix =
            "uploaded report r1 (repository hotpath-rs/hotpath, benchmark meta, baseline r0)";

        // A regression is a warning in every mode: the check run fails it,
        // never the job.
        for mode in [UploadMode::Enabled, UploadMode::FailOnError] {
            let r = render(&uploaded(verdict(1, 0)), &env(true, mode), Some("meta"));
            assert_eq!(r.level, Level::Warning);
            assert_eq!(
                r.message,
                format!("{prefix}; verdict: 1 regression; diff: {DASHBOARD_URL}")
            );
        }

        // Broken budgets alone are a regression.
        let r = render(
            &uploaded(verdict(0, 2)),
            &env(true, UploadMode::FailOnError),
            Some("meta"),
        );
        assert_eq!(r.level, Level::Warning);
        assert_eq!(
            r.message,
            format!("{prefix}; verdict: 2 budgets broken; diff: {DASHBOARD_URL}")
        );

        // Nothing judged never fails, and neither does a clean verdict.
        let not_judged = Verdict {
            judged: false,
            ..verdict(0, 0)
        };
        let r = render(
            &uploaded(not_judged),
            &env(true, UploadMode::FailOnError),
            Some("meta"),
        );
        assert_eq!(r.level, Level::Notice);
        assert_eq!(
            r.message,
            format!("{prefix}; verdict: not judged; diff: {DASHBOARD_URL}")
        );
        assert_eq!(
            render(
                &uploaded(verdict(0, 0)),
                &env(true, UploadMode::FailOnError),
                Some("meta")
            )
            .level,
            Level::Notice
        );
    }

    #[test]
    fn render_regressed_with_comment_error_carries_both() {
        let outcome = uploaded_as(
            UploadCreated {
                comment: CommentOutcome {
                    error: Some("the installation is suspended".into()),
                    ..CommentOutcome::default()
                },
                verdict: verdict(1, 0),
                ..created()
            },
            None,
        );
        let prefix =
            "uploaded report r1 (repository hotpath-rs/hotpath, benchmark meta, baseline r0)";
        let r = render(&outcome, &env(true, UploadMode::FailOnError), Some("meta"));
        assert_eq!(r.level, Level::Warning);
        assert_eq!(
            r.message,
            format!(
                "{prefix}; verdict: 1 regression; comment failed: the installation is suspended; diff: {DASHBOARD_URL}"
            )
        );
    }

    #[test]
    fn response_file_follows_the_outcome() {
        let dir = scratch_dir("response-file");
        let path = dir.join("response.json");
        let failed = Outcome::Failed {
            message: "connection refused".into(),
            body: None,
        };

        // Nothing to remove is not an error.
        store_response(&failed, &path).unwrap();
        assert!(!path.exists());

        // An upload overwrites whatever was there with the body as received,
        // a field this client does not know and the server's spacing included.
        std::fs::write(&path, "x".repeat(4096)).unwrap();
        let body = format!(
            r#"{{"id":"r1", "repository":"a/b","benchmark":"meta","new_field":{{"b":1,"a":2}},{VERDICT_FIELDS}}}"#
        );
        let outcome = interpret(201, None, body.clone());
        assert!(matches!(outcome, Outcome::Uploaded { .. }), "{outcome:?}");
        store_response(&outcome, &path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body);

        // A failed or skipped upload leaves no verdict of an earlier run behind.
        store_response(&failed, &path).unwrap();
        assert!(!path.exists());
        std::fs::write(&path, "{}").unwrap();
        store_response(&Outcome::Skipped("no token".into()), &path).unwrap();
        assert!(!path.exists());

        // An unwritable path is an error for the caller to warn about.
        let unwritable = dir.join("missing").join("response.json");
        assert!(store_response(&uploaded(verdict(0, 0)), &unwritable).is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn render_uploaded_with_comment_error_is_one_warning() {
        let outcome = uploaded_as(
            UploadCreated {
                comment: CommentOutcome {
                    error: Some("approve \"Pull requests: write\" for the installation".into()),
                    ..CommentOutcome::default()
                },
                ..created()
            },
            None,
        );
        let r = render(&outcome, &env(true, UploadMode::Enabled), Some("meta"));
        assert_eq!(r.level, Level::Warning);
        assert_eq!(
            r.message,
            format!("uploaded report r1 (repository hotpath-rs/hotpath, benchmark meta, baseline r0); verdict: no regressions; comment failed: approve \"Pull requests: write\" for the installation; diff: {DASHBOARD_URL}")
        );

        // A comment that was posted, or nothing to post, is a plain notice.
        for comment in [
            CommentOutcome {
                url: Some("https://github.com/c/1".into()),
                ..CommentOutcome::default()
            },
            CommentOutcome::default(),
        ] {
            let outcome = uploaded_as(
                UploadCreated {
                    comment,
                    ..created()
                },
                None,
            );
            let r = render(&outcome, &env(true, UploadMode::Enabled), Some("meta"));
            assert_eq!(r.level, Level::Notice);
            assert!(!r.message.contains("comment"));
        }

        // A skipped comment says why, and is a notice too.
        let outcome = uploaded_as(
            UploadCreated {
                comment: CommentOutcome {
                    skipped: Some("nothing to report".into()),
                    ..CommentOutcome::default()
                },
                ..created()
            },
            None,
        );
        let r = render(&outcome, &env(true, UploadMode::Enabled), Some("meta"));
        assert_eq!(r.level, Level::Notice);
        assert_eq!(
            r.message,
            format!("uploaded report r1 (repository hotpath-rs/hotpath, benchmark meta, baseline r0); verdict: no regressions; comment skipped: nothing to report; diff: {DASHBOARD_URL}")
        );
    }

    #[test]
    fn render_skipped_never_fails() {
        let outcome = Outcome::Skipped(
            "not in GitHub Actions or missing `id-token: write` permission".into(),
        );
        for (actions, mode) in [
            (false, UploadMode::Enabled),
            (true, UploadMode::Enabled),
            (true, UploadMode::FailOnError),
        ] {
            let r = render(&outcome, &env(actions, mode), Some("meta"));
            assert_eq!(r.level, Level::Notice);
            assert_eq!(
                r.message,
                "upload skipped: not in GitHub Actions or missing `id-token: write` permission"
            );
            assert_eq!(
                r.summary,
                "## hotpath.rs meta benchmark\n\nnotice: hotpath: upload skipped: not in GitHub Actions or missing `id-token: write` permission\n"
            );
        }
        assert!(render(&outcome, &env(true, UploadMode::FailOnError), None)
            .summary
            .starts_with("## hotpath.rs upload\n"));
    }

    #[test]
    fn render_failed_levels() {
        let outcome = Outcome::Failed {
            message: "request to https://hotpath.rs failed: connection refused".into(),
            body: None,
        };
        let r = render(&outcome, &env(true, UploadMode::Enabled), Some("meta"));
        assert_eq!(r.level, Level::Warning);
        assert_eq!(
            r.message,
            "upload failed: request to https://hotpath.rs failed: connection refused"
        );
        assert_eq!(
            r.summary,
            "## hotpath.rs meta benchmark\n\nwarning: hotpath: upload failed: request to https://hotpath.rs failed: connection refused\n"
        );
        assert_eq!(
            render(&outcome, &env(true, UploadMode::FailOnError), Some("meta")).level,
            Level::Error
        );
        // Outside Actions the level still follows the mode; only emission differs.
        assert_eq!(
            render(&outcome, &env(false, UploadMode::Enabled), Some("meta")).level,
            Level::Warning
        );
        assert_eq!(
            render(&outcome, &env(false, UploadMode::FailOnError), Some("meta")).level,
            Level::Error
        );

        // A server body goes into the summary's fenced block; an empty one does not.
        let rejected = interpret(403, None, r#"{"error":"not installed"}"#.into());
        let r = render(&rejected, &env(true, UploadMode::Enabled), Some("meta"));
        assert_eq!(r.message, "upload failed: not installed (HTTP 403)");
        assert!(
            r.summary
                .ends_with("\n```\n{\"error\":\"not installed\"}\n```\n"),
            "{}",
            r.summary
        );
        let timeout = interpret(408, Some("deadbeef".into()), String::new());
        assert!(
            !render(&timeout, &env(true, UploadMode::Enabled), Some("meta"))
                .summary
                .contains("```")
        );
    }

    #[test]
    fn escape_annotation_keeps_one_line() {
        assert_eq!(escape_annotation("plain"), "plain");
        assert_eq!(
            escape_annotation("100% of\r\nlines\nbreak"),
            "100%25 of%0D%0Alines%0Abreak"
        );
    }
}
