//! Builds the report `meta` object: toolchain, OS, timestamp, the source-root
//! prefix the server needs to map relative `location.file` values back to
//! repository paths, and - with the `hotpath-cloud` feature - the git/CI
//! provenance that makes a report self-describing, plus the policy file of
//! the checkout, which the server judges the report under.

use std::path::{Component, Path, PathBuf};
use time::OffsetDateTime;

use crate::json::policy_file::find_git_root;

/// Where `#[hotpath::main]` was compiled: the user package's
/// `CARGO_MANIFEST_DIR` and the main fn's `file!()`, which cargo makes
/// relative to the build workspace root.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SourceAnchor {
    pub(crate) manifest_dir: &'static str,
    pub(crate) main_file: &'static str,
}

pub(crate) fn build_meta(
    run: crate::json::JsonRun,
    criterion: Option<crate::json::JsonCriterion>,
    anchor: Option<SourceAnchor>,
) -> crate::json::JsonMeta {
    let source_root = source_root(anchor);

    // Fail loudly on upload runs: broken source links would otherwise only
    // surface as dead links on the server.
    #[cfg(feature = "hotpath-cloud")]
    if source_root.is_none()
        && crate::lib_on::cloud::enabled()
        && crate::lib_on::locations::any_relative_file().is_some()
    {
        eprintln!(
            "hotpath: could not resolve the source checkout from the working directory; \
             set HOTPATH_SOURCE_ROOT to the workspace path relative to the repo root \
             (source links will be unavailable in this report)"
        );
    }

    cfg_if::cfg_if! {
        if #[cfg(feature = "hotpath-cloud")] {
            let ci = crate::lib_on::ci_info::detect();
            let git_root = repository_root(
                checkout_git_root(anchor),
                std::env::current_dir().ok().as_deref(),
            );
            let git = report_git_info(git_root.as_deref(), ci.as_ref());
            // Set on every report, not only uploads, so a saved JSON says
            // which benchmark it is; the upload path reports invalid names.
            let benchmark = crate::lib_on::cloud::benchmark_name().ok();
            let ci = ci.map(|ci| ci.ci);
            // On every report, not only uploads, so the report file a relay
            // job posts carries the policy too.
            let policy = checkout_policy(
                git_root.as_deref(),
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
        rustc: env!("HOTPATH_RUSTC_VERSION").to_string(),
        os: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        created_at: now_whole_seconds(),
        source_root,
        git,
        ci,
        benchmark,
        policy,
        run: Some(run),
        criterion,
    }
}

/// The git root of the verified source checkout: the one the report's
/// relative `location.file` values resolve against.
#[cfg(feature = "hotpath-cloud")]
fn checkout_git_root(anchor: Option<SourceAnchor>) -> Option<PathBuf> {
    // A `HOTPATH_SOURCE_ROOT` override asserts the runtime checkout is the
    // source checkout, so its git root is used even when checkout resolution
    // fails.
    if std::env::var("HOTPATH_SOURCE_ROOT").is_ok() {
        find_git_root(&std::env::current_dir().ok()?)
    } else {
        Some(resolve_checkout(anchor)?.git_root)
    }
}

/// Where the commit identity is read and the policy file looked up: the
/// verified checkout's root, else the git root above the working directory.
/// Only source links need a verified checkout.
#[cfg(feature = "hotpath-cloud")]
fn repository_root(checkout_git_root: Option<PathBuf>, cwd: Option<&Path>) -> Option<PathBuf> {
    checkout_git_root.or_else(|| find_git_root(cwd?))
}

#[cfg(feature = "hotpath-cloud")]
fn report_git_info(
    git_root: Option<&Path>,
    ci: Option<&crate::lib_on::ci_info::CiContext>,
) -> Option<crate::json::JsonGitInfo> {
    let local = git_root.and_then(crate::lib_on::git_info::read_git_info_at);
    merge_git_info(local, ci)
}

/// The policy file of the checkout, as written; the client never parses it.
/// Without one the report is still written but its upload is refused. A
/// missing file is reported here only on a CI run that does not upload itself
/// (a relay's benchmark job); an upload run reports it as its outcome.
#[cfg(feature = "hotpath-cloud")]
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
#[cfg(feature = "hotpath-cloud")]
fn merge_git_info(
    local: Option<crate::json::JsonGitInfo>,
    ci: Option<&crate::lib_on::ci_info::CiContext>,
) -> Option<crate::json::JsonGitInfo> {
    let Some(ci) = ci else {
        return local;
    };
    let (sha, local_ref, repository, committed_at) = match local {
        Some(git) => (Some(git.sha), git.r#ref, git.repository, git.committed_at),
        None => (None, None, None, None),
    };
    // The local sha wins whenever there is one, so `committed_at` (read for
    // that sha) always describes the final sha; a sha only the environment
    // names gets no timestamp.
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
        committed_at,
    })
}

/// Build workspace root relative to the enclosing git root: the prefix to
/// prepend to relative `location.file` values ("" when the workspace root is
/// the repo root). `HOTPATH_SOURCE_ROOT` overrides; `None` when the checkout
/// cannot be resolved - an unverified value would produce broken links, so
/// the field is omitted instead of guessed.
fn source_root(anchor: Option<SourceAnchor>) -> Option<String> {
    if let Ok(v) = std::env::var("HOTPATH_SOURCE_ROOT") {
        return Some(v);
    }
    let checkout = resolve_checkout(anchor)?;
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
/// from (the build workspace root), not the runtime working directory.
/// `None` when the checkout cannot be verified (running outside the checkout,
/// deleted sources, an ambiguous anchor); `HOTPATH_SOURCE_ROOT` is the escape
/// hatch for those layouts.
fn resolve_checkout(anchor: Option<SourceAnchor>) -> Option<ResolvedCheckout> {
    resolve_checkout_at(&std::env::current_dir().ok()?, anchor)
}

/// With an anchor, the build workspace is the ancestor of the package
/// directory that holds the main file at its workspace-relative path, within
/// the git checkout around `cwd`: launched from above a nested workspace, the
/// walk up from `cwd` would never reach it. Without a match there (a binary
/// built in another checkout), the anchor's main file is looked up from `cwd`
/// upwards; without an anchor, the smallest registered relative file is.
fn resolve_checkout_at(cwd: &Path, anchor: Option<SourceAnchor>) -> Option<ResolvedCheckout> {
    let probe = match anchor {
        Some(anchor) => {
            let main_file = Path::new(anchor.main_file);
            // Only a plain relative path can anchor: `..`, a root or a drive
            // prefix leaves the workspace, and an absolute path joined onto
            // any directory matches it.
            let plain = main_file
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
            if !plain || main_file.as_os_str().is_empty() {
                return None;
            }
            if let Some(git_root) = find_git_root(cwd) {
                let manifest_dir = Path::new(anchor.manifest_dir);
                let mut workspaces = manifest_dir.ancestors().filter(|dir| {
                    manifest_dir
                        .strip_prefix(dir)
                        .is_ok_and(|package| main_file.starts_with(package))
                        && dir.join(main_file).is_file()
                        && find_git_root(dir).as_ref() == Some(&git_root)
                });
                if let Some(workspace_root) = workspaces.next() {
                    // Two candidates: either could be the wrong root.
                    if workspaces.next().is_some() {
                        return None;
                    }
                    return Some(ResolvedCheckout {
                        git_root,
                        workspace_root: workspace_root.to_path_buf(),
                    });
                }
            }
            anchor.main_file
        }
        None => crate::lib_on::locations::any_relative_file()?,
    };
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
    use std::path::{Path, PathBuf};

    use crate::lib_on::report_meta::{now_whole_seconds, resolve_checkout_at, SourceAnchor};
    use time::format_description::well_known::Rfc3339;

    /// A checkout under the temp dir with `.git` and empty `files`.
    fn checkout(name: &str, files: &[&str]) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("hotpath-report-meta-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        root
    }

    fn anchor(package: &Path, main_file: &'static str) -> SourceAnchor {
        SourceAnchor {
            manifest_dir: Box::leak(package.to_str().unwrap().into()),
            main_file,
        }
    }

    fn workspace_root(cwd: &Path, anchor: SourceAnchor) -> Option<PathBuf> {
        resolve_checkout_at(cwd, Some(anchor)).map(|c| c.workspace_root)
    }

    /// The anchor names the build workspace, not the package directory, for a
    /// root package, a workspace member and a nested workspace sharing the
    /// root crate's `src/main.rs`, all launched from the repository root.
    #[test]
    fn anchor_resolves_the_build_workspace() {
        let root = checkout(
            "anchor",
            &["src/main.rs", "crates/app/src/main.rs", "bench/src/main.rs"],
        );
        let cases = [
            (anchor(&root, "src/main.rs"), root.clone()),
            (
                anchor(&root.join("crates/app"), "crates/app/src/main.rs"),
                root.clone(),
            ),
            (
                anchor(&root.join("bench"), "src/main.rs"),
                root.join("bench"),
            ),
        ];
        for (anchor, expected) in cases {
            assert_eq!(workspace_root(&root, anchor), Some(expected), "{anchor:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Two candidate workspaces resolve nothing, even though the walk up
    /// from the working directory would find one.
    #[test]
    fn ambiguous_anchor_omits_the_checkout() {
        let root = checkout("ambiguous", &["x/x/src/main.rs", "x/x/x/src/main.rs"]);
        let ambiguous = anchor(&root.join("x/x"), "x/src/main.rs");
        assert_eq!(workspace_root(&root.join("x"), ambiguous), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A main file that is empty, absolute or leaves the workspace anchors
    /// nothing, even where the file exists and a walk would match.
    #[test]
    fn unsupported_anchor_paths_omit_the_checkout() {
        let root = checkout("unsupported", &["src/main.rs"]);
        let absolute: &'static str = Box::leak(root.join("src/main.rs").to_str().unwrap().into());
        for main_file in ["", absolute, "../src/main.rs", "src/../../src/main.rs"] {
            let unsupported = anchor(&root.join("src"), main_file);
            assert_eq!(workspace_root(&root, unsupported), None, "{main_file}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn created_at_is_utc_whole_seconds() {
        let now = now_whole_seconds();
        assert!(now.offset().is_utc());
        let wire = now.format(&Rfc3339).unwrap();
        assert!(wire.ends_with('Z') && !wire.contains('.'), "{wire}");
    }

    #[cfg(feature = "hotpath-cloud")]
    mod cloud {
        use crate::json::{
            JsonCiInfo, JsonGitInfo, JsonMeta, JsonPolicy, JsonPullRequest, JsonRun,
        };
        use crate::lib_on::ci_info::CiContext;
        use crate::lib_on::git_info::test_repo::{commit, git, temp_repo};
        use crate::lib_on::report_meta::{merge_git_info, report_git_info, repository_root};
        use time::macros::datetime;

        const LOCAL_SHA: &str = "1111111111111111111111111111111111111111";
        const CI_SHA: &str = "2222222222222222222222222222222222222222";
        const BASE_SHA: &str = "3333333333333333333333333333333333333333";
        const HEAD_SHA: &str = "4444444444444444444444444444444444444444";
        const COMMITTED_AT: time::OffsetDateTime = datetime!(2026-11-15 14:30:00 UTC);

        fn detached_local(sha: &str) -> JsonGitInfo {
            JsonGitInfo {
                sha: sha.to_string(),
                r#ref: None,
                base_sha: None,
                repository: Some("hotpath-rs/hotpath".to_string()),
                committed_at: Some(COMMITTED_AT),
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
            // The timestamp was read for the checked-out sha, which won.
            assert_eq!(git.committed_at, Some(COMMITTED_AT));
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
            // Nothing read the commit the environment names.
            assert_eq!(git.committed_at, None);

            assert!(merge_git_info(None, None).is_none());
        }

        /// Criterion-only benchmarks have no repo-relative locations, so the
        /// checkout is unverified.
        #[test]
        fn unverified_checkout_reports_the_built_commit() {
            let root = temp_repo("unverified-checkout");
            git(&root, &["init", "-q", "-b", "main"], &[]);
            let head = commit(
                &root,
                "2020-01-01T00:00:00+00:00",
                "2026-11-15T15:30:00+01:00",
            );
            commit(
                &root,
                "2020-01-02T00:00:00+00:00",
                "2026-11-16T09:00:00+00:00",
            );
            git(&root, &["checkout", "-q", "--detach", &head], &[]);
            let cwd = root.join("crates/torin");
            std::fs::create_dir_all(&cwd).unwrap();
            let git_root = repository_root(None, Some(&cwd));
            assert_eq!(git_root.as_deref(), Some(root.as_path()));

            let git = report_git_info(git_root.as_deref(), Some(&pull_request_ci(CI_SHA)))
                .expect("git info");
            assert_eq!(git.sha, head);
            assert_eq!(git.committed_at, Some(COMMITTED_AT));
            assert_eq!(git.r#ref, None);
            assert_eq!(git.base_sha.as_deref(), Some(BASE_SHA));

            let push_ci = CiContext {
                ci: JsonCiInfo {
                    provider: "github-actions".to_string(),
                    event: "push".to_string(),
                    ..JsonCiInfo::default()
                },
                sha: Some(head.clone()),
                r#ref: Some("refs/heads/main".to_string()),
                repository: Some("hotpath-rs/hotpath".to_string()),
                base_sha: None,
            };
            let git = report_git_info(git_root.as_deref(), Some(&push_ci)).expect("git info");
            assert_eq!(git.sha, head);
            assert_eq!(git.committed_at, Some(COMMITTED_AT));
            assert_eq!(git.r#ref.as_deref(), Some("refs/heads/main"));
            assert_eq!(git.base_sha, None);

            let verified = root.join("verified");
            assert_eq!(
                repository_root(Some(verified.clone()), Some(&cwd)),
                Some(verified)
            );
            let _ = std::fs::remove_dir_all(&root);
        }

        #[test]
        fn no_checkout_falls_back_to_ci() {
            let cwd = temp_repo("no-checkout");
            let git_root = repository_root(None, Some(&cwd));
            assert_eq!(
                git_root, None,
                "temp dir must not sit inside a git checkout"
            );
            let git = report_git_info(git_root.as_deref(), Some(&pull_request_ci(CI_SHA)))
                .expect("git info");
            assert_eq!(git.sha, CI_SHA);
            assert_eq!(git.committed_at, None);
            assert_eq!(git.r#ref.as_deref(), Some("refs/pull/42/merge"));
            assert!(report_git_info(None, None).is_none());
            let _ = std::fs::remove_dir_all(&cwd);
        }

        #[test]
        fn absent_fields_add_no_keys() {
            let meta = JsonMeta {
                rustc: "1.89.0".to_string(),
                os: "macos-aarch64".to_string(),
                created_at: datetime!(2026-08-27 10:15:42 UTC),
                source_root: Some(String::new()),
                git: Some(JsonGitInfo {
                    committed_at: None,
                    ..detached_local(LOCAL_SHA)
                }),
                ci: None,
                benchmark: None,
                policy: None,
                run: None,
                criterion: None,
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
                criterion: None,
            };
            let sent = meta.policy.clone();
            let json = serde_json::to_string(&meta).unwrap();
            let wire: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(wire["git"]["committed_at"], "2026-11-15T14:30:00Z");
            let back: JsonMeta = serde_json::from_str(&json).unwrap();

            let git = back.git.expect("git info");
            assert_eq!(git.sha, LOCAL_SHA);
            assert_eq!(git.base_sha.as_deref(), Some(BASE_SHA));
            assert_eq!(git.repository.as_deref(), Some("hotpath-rs/hotpath"));
            assert_eq!(git.committed_at, Some(COMMITTED_AT));
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
            assert_eq!(git.committed_at, None);
            assert!(meta.ci.is_none());
            assert!(meta.benchmark.is_none());
            assert!(meta.policy.is_none());
        }
    }
}
