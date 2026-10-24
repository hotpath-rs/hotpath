//! Builds the report `meta` object: toolchain, OS, timestamp, the source-root
//! prefix the server needs to map relative `location.file` values back to
//! repository paths, and - with the `hotpath-cloud-meta` feature - the git/CI
//! provenance that makes a report self-describing, plus the policy file of
//! the checkout, which the server judges the report under.

use std::path::{Path, PathBuf};
use time::OffsetDateTime;

use crate::json::policy_file::find_git_root;

pub(crate) fn build_meta(run: crate::json::JsonRun) -> crate::json::JsonMeta {
    let source_root = source_root();

    // Fail loudly on upload runs: broken source links would otherwise only
    // surface as dead links on the server.
    #[cfg(feature = "hotpath-cloud-meta")]
    if source_root.is_none()
        && crate::lib_on::cloud::enabled()
        && crate::lib_on::locations::any_relative_file().is_some()
    {
        eprintln!(
            "hotpath: could not resolve the source checkout from the working directory; \
             set HOTPATH_META_SOURCE_ROOT to the workspace path relative to the repo root \
             (source links will be unavailable in this report)"
        );
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "hotpath-cloud-meta")] {
            let ci = crate::lib_on::ci_info::detect();
            let git_root = checkout_git_root();
            let local = git_root
                .as_deref()
                .and_then(crate::lib_on::git_info::read_git_info_at);
            let git = merge_git_info(local, ci.as_ref());
            // Set on every report, not only uploads, so a saved JSON says
            // which benchmark it is; the upload path reports invalid names.
            let benchmark = crate::lib_on::cloud::benchmark_name().ok();
            let ci = ci.map(|ci| ci.ci);
            // On every report, not only uploads, so the report file a relay
            // job posts carries the policy too.
            let policy = checkout_policy(
                policy_root(git_root).as_deref(),
                benchmark.as_deref(),
                ci.is_some(),
            );
        } else {
            let git: Option<crate::json::JsonGitInfo> = None;
            let ci: Option<crate::json::JsonCiInfo> = None;
            let benchmark: Option<String> = None;
            let policy: Option<crate::json::JsonPolicy> = None;
        }
    }

    crate::json::JsonMeta {
        rustc: env!("HOTPATH_META_RUSTC_VERSION").to_string(),
        os: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        created_at: now_whole_seconds(),
        source_root,
        git,
        ci,
        benchmark,
        policy,
        run: Some(run),
    }
}

/// The git root of the checkout the report describes: where its git identity
/// is read and its policy file looked up.
#[cfg(feature = "hotpath-cloud-meta")]
fn checkout_git_root() -> Option<PathBuf> {
    // A `HOTPATH_META_SOURCE_ROOT` override asserts the runtime checkout is
    // the source checkout, so its git identity is trusted even when checkout
    // resolution fails.
    if std::env::var("HOTPATH_META_SOURCE_ROOT").is_ok() {
        find_git_root(&std::env::current_dir().ok()?)
    } else {
        Some(resolve_checkout()?.git_root)
    }
}

/// Where the policy file is looked up: the checkout's git root, else the git
/// root above the working directory. An unverified checkout (no registered
/// source locations, or a nested workspace started from the repository root)
/// withholds source links and the commit identity, not the policy.
#[cfg(feature = "hotpath-cloud-meta")]
fn policy_root(checkout_git_root: Option<PathBuf>) -> Option<PathBuf> {
    checkout_git_root.or_else(|| find_git_root(&std::env::current_dir().ok()?))
}

/// The policy file of the checkout, as written; the client never parses it.
/// Without one the report is still written but its upload is refused. A
/// missing file is reported here only on a CI run that does not upload itself
/// (a relay's benchmark job); an upload run reports it as its outcome.
#[cfg(feature = "hotpath-cloud-meta")]
fn checkout_policy(
    git_root: Option<&Path>,
    benchmark: Option<&str>,
    in_ci: bool,
) -> Option<crate::json::JsonPolicy> {
    use crate::json::policy_file::PolicyLookup;

    match crate::json::policy_file::lookup(git_root, benchmark) {
        PolicyLookup::NotFound => {
            let uploading = crate::lib_on::cloud::enabled();
            if git_root.is_none() && (uploading || in_ci) {
                eprintln!(
                    "hotpath: no git repository found from the working directory, so no policy \
                     file was looked for."
                );
            }
            if in_ci && !uploading {
                eprintln!(
                    "hotpath: {}",
                    crate::lib_on::cloud::missing_policy_message(benchmark)
                );
            }
            None
        }
        PolicyLookup::Found(policy) => Some(policy),
        PolicyLookup::Unusable(unusable) => {
            eprintln!("hotpath: {unusable} The report carries no policy.");
            None
        }
    }
}

/// The checkout is the commit identity; the environment only describes the
/// run. A job may build something other than the event commit - a mid-job
/// `git checkout <base sha>`, or `actions/checkout` with
/// `ref: <pull_request.head.sha>` - while `GITHUB_SHA` stays pinned to the
/// merge commit, so trusting it attributes measurements to a commit that was
/// never compiled.
///
/// `GITHUB_REF` applies only when the environment does describe the
/// checked-out commit, since it names that commit and no other; a default
/// pull request checkout is detached, so then it is the only source of a ref.
///
/// The event's base sha needs a narrower guard than the ref does. It is the
/// right baseline for the merge commit and for the head commit alike, so a
/// job that built either is still measured against it; only a job that built
/// the base commit itself has no base to compare against.
#[cfg(feature = "hotpath-cloud-meta")]
fn merge_git_info(
    local: Option<crate::json::JsonGitInfo>,
    ci: Option<&crate::lib_on::ci_info::CiContext>,
) -> Option<crate::json::JsonGitInfo> {
    let Some(ci) = ci else {
        return local;
    };
    let (sha, local_ref, repository) = match local {
        Some(git) => (Some(git.sha), git.r#ref, git.repository),
        None => (None, None, None),
    };
    let sha = sha.or_else(|| ci.sha.clone())?;
    let describes_checkout = ci.sha.as_deref() == Some(sha.as_str());

    Some(crate::json::JsonGitInfo {
        r#ref: if describes_checkout {
            ci.r#ref.clone().or(local_ref)
        } else {
            local_ref
        },
        base_sha: ci.base_sha.clone().filter(|base| base != &sha),
        repository: repository.or_else(|| ci.repository.clone()),
        sha,
    })
}

/// Build workspace root relative to the enclosing git root: the prefix to
/// prepend to relative `location.file` values ("" when the workspace root is
/// the repo root). `HOTPATH_META_SOURCE_ROOT` overrides; `None` when the
/// checkout cannot be resolved - an unverified value would produce broken
/// links, so the field is omitted instead of guessed.
fn source_root() -> Option<String> {
    if let Ok(v) = std::env::var("HOTPATH_META_SOURCE_ROOT") {
        return Some(v);
    }
    let checkout = resolve_checkout()?;
    let rel = checkout
        .workspace_root
        .strip_prefix(&checkout.git_root)
        .ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// The checkout the report's relative `location.file` values were compiled
/// from, verified against the filesystem rather than guessed from the
/// runtime working directory.
struct ResolvedCheckout {
    git_root: PathBuf,
    workspace_root: PathBuf,
}

/// Relative `file!()` paths are relative to the directory cargo invoked rustc
/// from (the build workspace root), not the runtime working directory. The
/// workspace root is located as the nearest ancestor of the working directory
/// that actually contains one of the registered relative source files, and
/// the git root as its enclosing checkout - so both are verified to belong to
/// the sources in the report. `None` when nothing relative is registered or
/// no ancestor matches (a nested workspace launched from above it, running
/// outside the checkout, deleted sources); `HOTPATH_META_SOURCE_ROOT` is the
/// escape hatch for those layouts.
fn resolve_checkout() -> Option<ResolvedCheckout> {
    let cwd = std::env::current_dir().ok()?;
    let probe = crate::lib_on::locations::any_relative_file()?;
    let workspace_root = cwd
        .ancestors()
        .find(|dir| dir.join(probe).is_file())
        .map(Path::to_path_buf)?;
    let git_root = find_git_root(&workspace_root)?;
    Some(ResolvedCheckout {
        git_root,
        workspace_root,
    })
}

/// Whole seconds, so the wire value is `2026-08-27T10:15:42Z`.
fn now_whole_seconds() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    now.replace_nanosecond(0).unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use crate::lib_on::report_meta::now_whole_seconds;
    use time::format_description::well_known::Rfc3339;

    #[test]
    fn created_at_is_utc_whole_seconds() {
        let now = now_whole_seconds();
        assert!(now.offset().is_utc());
        let wire = now.format(&Rfc3339).unwrap();
        assert!(wire.ends_with('Z') && !wire.contains('.'), "{wire}");
    }

    #[cfg(feature = "hotpath-cloud-meta")]
    mod cloud {
        use crate::json::{
            JsonCiInfo, JsonGitInfo, JsonMeta, JsonPolicy, JsonPullRequest, JsonRun,
        };
        use crate::lib_on::ci_info::CiContext;
        use crate::lib_on::report_meta::merge_git_info;
        use time::macros::datetime;

        const LOCAL_SHA: &str = "1111111111111111111111111111111111111111";
        const CI_SHA: &str = "2222222222222222222222222222222222222222";
        const BASE_SHA: &str = "3333333333333333333333333333333333333333";
        const HEAD_SHA: &str = "4444444444444444444444444444444444444444";

        fn detached_local(sha: &str) -> JsonGitInfo {
            JsonGitInfo {
                sha: sha.to_string(),
                r#ref: None,
                base_sha: None,
                repository: Some("hotpath-rs/hotpath".to_string()),
            }
        }

        /// A default pull request run: the runner checks out the merge commit
        /// `GITHUB_SHA` names, detached.
        fn pull_request_ci(sha: &str) -> CiContext {
            CiContext {
                ci: JsonCiInfo {
                    provider: "github-actions".to_string(),
                    event: "pull_request".to_string(),
                    pull_request: Some(JsonPullRequest {
                        number: 42,
                        base_ref: "main".to_string(),
                        head_ref: "feature-x".to_string(),
                        head_sha: Some(HEAD_SHA.to_string()),
                    }),
                    ..JsonCiInfo::default()
                },
                sha: Some(sha.to_string()),
                r#ref: Some("refs/pull/42/merge".to_string()),
                repository: Some("pawurb/other-name".to_string()),
                base_sha: Some(BASE_SHA.to_string()),
            }
        }

        #[test]
        fn env_describes_the_commit_it_checked_out() {
            let git = merge_git_info(
                Some(detached_local(LOCAL_SHA)),
                Some(&pull_request_ci(LOCAL_SHA)),
            )
            .expect("git info");
            assert_eq!(git.sha, LOCAL_SHA);
            // The checkout is detached, so only the environment names a ref.
            assert_eq!(git.r#ref.as_deref(), Some("refs/pull/42/merge"));
            assert_eq!(git.base_sha.as_deref(), Some(BASE_SHA));
            assert_eq!(git.repository.as_deref(), Some("hotpath-rs/hotpath"));
        }

        /// A job that built the head commit rather than the merge commit:
        /// `GITHUB_REF` names a commit that did not run, but the event's base
        /// is still what the head should be compared against.
        #[test]
        fn head_checkout_drops_the_env_ref_but_keeps_the_base() {
            let git = merge_git_info(
                Some(detached_local(LOCAL_SHA)),
                Some(&pull_request_ci(CI_SHA)),
            )
            .expect("git info");
            assert_eq!(git.sha, LOCAL_SHA);
            assert_eq!(git.r#ref, None);
            assert_eq!(git.base_sha.as_deref(), Some(BASE_SHA));
        }

        /// A job that checked out the base commit is measuring the base, so it
        /// has nothing to be compared against.
        #[test]
        fn base_checkout_drops_the_base() {
            let git = merge_git_info(
                Some(detached_local(BASE_SHA)),
                Some(&pull_request_ci(CI_SHA)),
            )
            .expect("git info");
            assert_eq!(git.sha, BASE_SHA);
            assert_eq!(git.base_sha, None);
        }

        #[test]
        fn checkout_fills_gaps_the_provider_leaves() {
            let bare_ci = CiContext {
                ci: JsonCiInfo {
                    provider: "github-actions".to_string(),
                    ..JsonCiInfo::default()
                },
                sha: None,
                r#ref: None,
                repository: None,
                base_sha: None,
            };
            let local = JsonGitInfo {
                r#ref: Some("refs/heads/main".to_string()),
                ..detached_local(LOCAL_SHA)
            };
            let git = merge_git_info(Some(local), Some(&bare_ci)).expect("git info");
            assert_eq!(git.sha, LOCAL_SHA);
            assert_eq!(git.r#ref.as_deref(), Some("refs/heads/main"));
            assert_eq!(git.repository.as_deref(), Some("hotpath-rs/hotpath"));
        }

        /// An unreadable `.git` leaves the environment as the only source, and
        /// then it does describe the checkout by default.
        #[test]
        fn ci_alone_still_yields_git_info() {
            let git = merge_git_info(None, Some(&pull_request_ci(CI_SHA))).expect("git info");
            assert_eq!(git.sha, CI_SHA);
            assert_eq!(git.r#ref.as_deref(), Some("refs/pull/42/merge"));
            assert_eq!(git.base_sha.as_deref(), Some(BASE_SHA));
            assert_eq!(git.repository.as_deref(), Some("pawurb/other-name"));

            assert!(merge_git_info(None, None).is_none());
        }

        #[test]
        fn absent_fields_add_no_keys() {
            let meta = JsonMeta {
                rustc: "1.89.0".to_string(),
                os: "macos-aarch64".to_string(),
                created_at: datetime!(2026-08-27 10:15:42 UTC),
                source_root: Some(String::new()),
                git: Some(detached_local(LOCAL_SHA)),
                ci: None,
                benchmark: None,
                policy: None,
                run: None,
            };
            let value: serde_json::Value = serde_json::to_value(&meta).unwrap();
            // Sorted: the `cloud` feature turns on `serde_json/preserve_order`.
            let keys = |value: &serde_json::Value| -> Vec<String> {
                let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
                keys.sort();
                keys
            };
            assert_eq!(
                keys(&value),
                ["created_at", "git", "os", "rustc", "source_root"]
            );
            assert_eq!(keys(&value["git"]), ["repository", "sha"]);
        }

        #[test]
        fn new_fields_round_trip() {
            let meta = JsonMeta {
                rustc: "1.89.0".to_string(),
                os: "macos-aarch64".to_string(),
                created_at: datetime!(2026-08-27 10:15:42 UTC),
                source_root: Some(String::new()),
                git: merge_git_info(
                    Some(detached_local(LOCAL_SHA)),
                    Some(&pull_request_ci(LOCAL_SHA)),
                ),
                ci: Some(pull_request_ci(LOCAL_SHA).ci),
                benchmark: Some("ci".to_string()),
                policy: Some(JsonPolicy {
                    source: "# ours\n[functions.timing]\n".to_string(),
                    path: "hotpath/ci-policy.toml".to_string(),
                }),
                run: Some(JsonRun::Merged),
            };
            let sent = meta.policy.clone();
            let json = serde_json::to_string(&meta).unwrap();
            let back: JsonMeta = serde_json::from_str(&json).unwrap();

            let git = back.git.expect("git info");
            assert_eq!(git.sha, LOCAL_SHA);
            assert_eq!(git.base_sha.as_deref(), Some(BASE_SHA));
            assert_eq!(git.repository.as_deref(), Some("hotpath-rs/hotpath"));
            let ci = back.ci.expect("ci info");
            assert_eq!(ci.provider, "github-actions");
            assert_eq!(ci.event, "pull_request");
            let pr = ci.pull_request.expect("pull request info");
            assert_eq!(pr.number, 42);
            assert_eq!(pr.base_ref, "main");
            assert_eq!(pr.head_ref, "feature-x");
            assert_eq!(pr.head_sha.as_deref(), Some(HEAD_SHA));
            assert_eq!(back.benchmark.as_deref(), Some("ci"));
            assert_eq!(back.run, Some(JsonRun::Merged));
            assert_eq!(back.policy, sent);
        }

        #[test]
        fn old_report_deserializes() {
            let json = r#"{
                "rustc": "1.89.0",
                "os": "macos-aarch64",
                "created_at": "2026-08-27T10:15:42Z",
                "source_root": "",
                "git": {"sha": "1111111111111111111111111111111111111111", "ref": "refs/heads/main"}
            }"#;
            let meta: JsonMeta = serde_json::from_str(json).unwrap();
            let git = meta.git.expect("git info");
            assert_eq!(git.base_sha, None);
            assert_eq!(git.repository, None);
            assert!(meta.ci.is_none());
            assert!(meta.benchmark.is_none());
            assert!(meta.policy.is_none());
        }
    }
}
