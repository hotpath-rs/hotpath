//! Reads CI provenance from the environment: the `meta.ci` object plus what
//! the environment claims about `meta.git`. Those claims hold only for a run
//! that built the commit the event names, so `report_meta::merge_git_info`
//! decides which of them survive.
//!
//! Every field is best-effort: a missing variable, an unreadable event file
//! or an unexpected payload shape omits a field, never fails the run.

use std::path::Path;

use serde::Deserialize;

use crate::json::{JsonCiInfo, JsonPullRequest};

/// The `meta.ci` object plus the commit identity the environment claims.
pub(crate) struct CiContext {
    pub(crate) ci: JsonCiInfo,
    pub(crate) sha: Option<String>,
    pub(crate) r#ref: Option<String>,
    pub(crate) repository: Option<String>,
    pub(crate) base_sha: Option<String>,
}

pub(crate) fn detect() -> Option<CiContext> {
    if env("GITHUB_ACTIONS").as_deref() != Some("true") {
        return None;
    }
    // Every Actions run names its event; without one there is no run to
    // describe, so `ci` is omitted rather than filled with a placeholder.
    Some(github_actions(env("GITHUB_EVENT_NAME")?))
}

fn github_actions(event: String) -> CiContext {
    let git_ref = env("GITHUB_REF");
    let is_pull_request = event == "pull_request";
    let payload = is_pull_request
        .then(|| env("GITHUB_EVENT_PATH").and_then(|path| read_pull_request(Path::new(&path))))
        .flatten();

    CiContext {
        ci: JsonCiInfo {
            provider: "github-actions".to_string(),
            event,
            pull_request: is_pull_request
                .then(|| pull_request(git_ref.as_deref(), payload.as_ref()))
                .flatten(),
            run_id: env("GITHUB_RUN_ID"),
            run_attempt: env("GITHUB_RUN_ATTEMPT"),
            workflow: env("GITHUB_WORKFLOW"),
            actor: env("GITHUB_ACTOR"),
            repository_id: env("GITHUB_REPOSITORY_ID"),
        },
        sha: env("GITHUB_SHA"),
        r#ref: git_ref,
        repository: env("GITHUB_REPOSITORY"),
        base_sha: payload.and_then(|pr| pr.base).and_then(|base| base.sha),
    }
}

/// `GITHUB_REF` (`refs/pull/<n>/merge`) is tried before the event file so all
/// three fields come from the environment. Otherwise a payload that failed to
/// parse would cost `base_ref` as well as `git.base_sha` - and `base_ref` is
/// the server's fallback for exactly that missing sha.
fn pull_request(git_ref: Option<&str>, payload: Option<&PullRequest>) -> Option<JsonPullRequest> {
    Some(JsonPullRequest {
        number: git_ref
            .and_then(pr_number_from_ref)
            .or_else(|| payload.and_then(|pr| pr.number))?,
        base_ref: env("GITHUB_BASE_REF")?,
        head_ref: env("GITHUB_HEAD_REF")?,
        head_sha: payload
            .and_then(|pr| pr.head.as_ref())
            .and_then(|head| head.sha.clone()),
    })
}

fn pr_number_from_ref(git_ref: &str) -> Option<u64> {
    git_ref
        .strip_prefix("refs/pull/")?
        .split('/')
        .next()?
        .parse()
        .ok()
}

#[derive(Deserialize)]
struct EventPayload {
    pull_request: Option<PullRequest>,
}

#[derive(Deserialize)]
struct PullRequest {
    number: Option<u64>,
    base: Option<EventCommit>,
    head: Option<EventCommit>,
}

#[derive(Deserialize)]
struct EventCommit {
    sha: Option<String>,
}

/// The runner writes `$GITHUB_EVENT_PATH` once, when the event fired, so the
/// base sha in it and `GITHUB_SHA` describe the same snapshot by
/// construction. Asking the REST API instead would answer with the pull
/// request's base as of now, which drifts while a benchmark runs.
fn read_pull_request(path: &Path) -> Option<PullRequest> {
    let contents = std::fs::read_to_string(path).ok()?;
    let payload: EventPayload = serde_json::from_str(&contents).ok()?;
    payload.pull_request
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use crate::lib_on::ci_info::{pr_number_from_ref, read_pull_request};
    use std::path::PathBuf;

    fn write_event(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hotpath-ci-info-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.json"));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn reads_pull_request_payload() {
        let path = write_event(
            "pull_request",
            r#"{
                "action": "synchronize",
                "number": 42,
                "pull_request": {
                    "number": 42,
                    "head": { "sha": "1111111111111111111111111111111111111111" },
                    "base": {
                        "ref": "main",
                        "sha": "2222222222222222222222222222222222222222",
                        "repo": { "full_name": "owner/name" }
                    }
                },
                "repository": { "full_name": "owner/name" }
            }"#,
        );

        let pr = read_pull_request(&path).expect("pull_request parsed");
        assert_eq!(pr.number, Some(42));
        assert_eq!(
            pr.base.and_then(|b| b.sha).as_deref(),
            Some("2222222222222222222222222222222222222222")
        );
        assert_eq!(
            pr.head.and_then(|h| h.sha).as_deref(),
            Some("1111111111111111111111111111111111111111")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn payload_without_head_still_parses() {
        let path = write_event(
            "no_head",
            r#"{"pull_request":{"number":42,"base":{"sha":"2222222222222222222222222222222222222222"}}}"#,
        );
        let pr = read_pull_request(&path).expect("pull_request parsed");
        assert_eq!(pr.number, Some(42));
        assert!(pr.head.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn push_payload_has_no_pull_request() {
        let path = write_event(
            "push",
            r#"{"ref":"refs/heads/main","after":"3333333333333333333333333333333333333333"}"#,
        );
        assert!(read_pull_request(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parses_pr_number_from_the_merge_ref() {
        assert_eq!(pr_number_from_ref("refs/pull/42/merge"), Some(42));
        assert_eq!(pr_number_from_ref("refs/pull/7/head"), Some(7));
        for git_ref in [
            "refs/heads/main",
            "refs/pull//merge",
            "refs/pull/not-a-number/merge",
            "refs/tags/v1.0",
            "",
        ] {
            assert_eq!(pr_number_from_ref(git_ref), None, "{git_ref}");
        }
    }

    #[test]
    fn tolerates_unexpected_shapes() {
        let missing = std::env::temp_dir().join("hotpath-ci-info-does-not-exist.json");
        assert!(read_pull_request(&missing).is_none());

        let invalid = write_event("invalid", "not json {");
        assert!(read_pull_request(&invalid).is_none());
        let _ = std::fs::remove_file(&invalid);

        let wrong_type = write_event("wrong_type", r#"{"pull_request":{"number":"42"}}"#);
        assert!(read_pull_request(&wrong_type).is_none());
        let _ = std::fs::remove_file(&wrong_type);

        let partial = write_event("partial", r#"{"pull_request":{"base":{}}}"#);
        let pr = read_pull_request(&partial).expect("pull_request parsed");
        assert_eq!(pr.number, None);
        assert!(pr.base.expect("base parsed").sha.is_none());
        let _ = std::fs::remove_file(&partial);
    }
}
