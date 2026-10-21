#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use hotpath::json::JsonMeta;

    use crate::common::example::{assert_success, Example};
    use crate::common::report::parse_report;

    const OTHER_SHA: &str = "2222222222222222222222222222222222222222";
    const BASE_SHA: &str = "3333333333333333333333333333333333333333";
    const PR_HEAD_SHA: &str = "4444444444444444444444444444444444444444";

    /// The commit the test process actually has checked out, i.e. what the
    /// profiled example measures. `GITHUB_SHA` only describes the checkout
    /// when it equals this.
    fn head_sha() -> String {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git rev-parse HEAD");
        assert!(output.status.success(), "git rev-parse HEAD failed");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn run_example(envs: &[(&str, &str)]) -> JsonMeta {
        run_example_in(None, envs).0
    }

    /// The report's `meta` and the run's stderr. With `cwd` the example runs
    /// from that directory, which stands for the checkout.
    ///
    /// cargo run -p test-all-features --example basic_all_features --features hotpath-cloud
    fn run_example_in(cwd: Option<&Path>, envs: &[(&str, &str)]) -> (JsonMeta, String) {
        run_named_example_in("basic_all_features", cwd, envs)
    }

    /// cargo run -p test-all-features --example no_locations --features hotpath-cloud
    fn run_named_example_in(
        example: &str,
        cwd: Option<&Path>,
        envs: &[(&str, &str)],
    ) -> (JsonMeta, String) {
        let mut run = Example::new("test-all-features", example)
            .manifest_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml"))
            .features("hotpath-cloud")
            .json()
            .env("HOTPATH_REPORT", "functions-timing")
            .env_remove("HOTPATH_UPLOAD")
            .env_remove("HOTPATH_BENCHMARK")
            .env_remove("HOTPATH_POLICY_PATH")
            .env_remove("HOTPATH_SOURCE_ROOT");
        if let Some(cwd) = cwd {
            run = run.current_dir(cwd);
        }
        for var in [
            "GITHUB_ACTIONS",
            "GITHUB_BASE_REF",
            "GITHUB_HEAD_REF",
            "GITHUB_SHA",
            "GITHUB_REF",
            "GITHUB_REPOSITORY",
            "GITHUB_REPOSITORY_ID",
            "GITHUB_EVENT_NAME",
            "GITHUB_EVENT_PATH",
            "GITHUB_RUN_ID",
            "GITHUB_WORKFLOW",
            "GITHUB_ACTOR",
        ] {
            run = run.env_remove(var);
        }

        let output = run.envs(envs).output();
        assert_success(&output);

        let meta = parse_report(&String::from_utf8_lossy(&output.stdout)).meta;
        (meta, String::from_utf8_lossy(&output.stderr).into_owned())
    }

    fn write_event_payload(name: &str, base_sha: &str) -> PathBuf {
        write_event(name, base_sha, Some(PR_HEAD_SHA))
    }

    /// `name` keeps concurrently running tests off each other's payload: they
    /// delete the file once their subprocess exits.
    fn write_event(name: &str, base_sha: &str, head_sha: Option<&str>) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hotpath-cloud-meta-event-{}-{name}.json",
            std::process::id()
        ));
        let head = head_sha
            .map(|sha| format!(r#","head":{{"sha":"{sha}"}}"#))
            .unwrap_or_default();
        std::fs::write(
            &path,
            format!(
                r#"{{"action":"synchronize","pull_request":{{"number":42,"base":{{"ref":"main","sha":"{base_sha}"}}{head}}}}}"#
            ),
        )
        .expect("event payload written");
        path
    }

    #[test]
    fn meta_outside_ci_describes_the_checkout() {
        let meta = run_example(&[]);

        assert!(
            meta.ci.is_none(),
            "no CI provider outside CI: {:?}",
            meta.ci
        );
        assert_eq!(meta.benchmark.as_deref(), Some("default"));

        let git = meta.git.expect("git info from the checkout");
        assert_eq!(git.sha.len(), 40, "sha: {}", git.sha);
        assert_eq!(git.repository.as_deref(), Some("hotpath-rs/hotpath"));
        // A merge base needs a commit-graph walk the `.git` reader avoids.
        assert_eq!(git.base_sha, None);
    }

    /// A default pull request run: `actions/checkout` leaves HEAD at the merge
    /// commit `GITHUB_SHA` names, so the environment does describe what was
    /// built and its ref and base sha apply.
    #[test]
    fn meta_in_github_actions_describes_the_run() {
        let head = head_sha();
        let event_path = write_event_payload("default-checkout", BASE_SHA);
        let meta = run_example(&[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", &head),
            ("GITHUB_REF", "refs/pull/42/merge"),
            ("GITHUB_REPOSITORY", "hotpath-rs/hotpath"),
            ("GITHUB_REPOSITORY_ID", "123456"),
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("GITHUB_EVENT_PATH", &event_path.to_string_lossy()),
            ("GITHUB_BASE_REF", "main"),
            ("GITHUB_HEAD_REF", "feature-x"),
            ("GITHUB_RUN_ID", "987654321"),
            ("GITHUB_WORKFLOW", "benchmarks"),
            ("GITHUB_ACTOR", "pawurb"),
            ("HOTPATH_BENCHMARK", "timing-linux"),
        ]);
        let _ = std::fs::remove_file(&event_path);

        let git = meta.git.expect("git info");
        assert_eq!(git.sha, head);
        assert_eq!(git.r#ref.as_deref(), Some("refs/pull/42/merge"));
        assert_eq!(git.base_sha.as_deref(), Some(BASE_SHA));
        assert_eq!(git.repository.as_deref(), Some("hotpath-rs/hotpath"));

        let ci = meta.ci.expect("ci info");
        assert_eq!(ci.provider, "github-actions");
        assert_eq!(ci.event, "pull_request");
        let pr = ci.pull_request.expect("pull request info");
        assert_eq!(pr.number, 42);
        assert_eq!(pr.base_ref, "main");
        assert_eq!(pr.head_ref, "feature-x");
        assert_eq!(pr.head_sha.as_deref(), Some(PR_HEAD_SHA));
        assert_eq!(ci.run_id.as_deref(), Some("987654321"));
        assert_eq!(ci.workflow.as_deref(), Some("benchmarks"));
        assert_eq!(ci.actor.as_deref(), Some("pawurb"));
        assert_eq!(ci.repository_id.as_deref(), Some("123456"));

        assert_eq!(meta.benchmark.as_deref(), Some("timing-linux"));
    }

    /// A job that checked out `pull_request.head.sha` to avoid benchmarking
    /// the synthetic merge commit: `GITHUB_SHA` names a commit that never ran,
    /// so its ref goes, but the event's base is still the head's baseline.
    #[test]
    fn head_checkout_wins_over_the_environment() {
        let event_path = write_event_payload("head-checkout", BASE_SHA);
        let meta = run_example(&[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", OTHER_SHA),
            ("GITHUB_REF", "refs/pull/42/merge"),
            ("GITHUB_REPOSITORY", "pawurb/renamed-repo"),
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("GITHUB_EVENT_PATH", &event_path.to_string_lossy()),
            ("GITHUB_BASE_REF", "main"),
            ("GITHUB_HEAD_REF", "feature-x"),
        ]);
        let _ = std::fs::remove_file(&event_path);

        let git = meta.git.expect("git info");
        assert_eq!(git.sha, head_sha());
        assert_ne!(git.r#ref.as_deref(), Some("refs/pull/42/merge"));
        assert_eq!(git.base_sha.as_deref(), Some(BASE_SHA));
        assert_eq!(git.repository.as_deref(), Some("hotpath-rs/hotpath"));

        // `ci.*` describes the run, not the checkout, so it stays intact.
        let ci = meta.ci.expect("ci info");
        assert_eq!(ci.event, "pull_request");
        let pr = ci.pull_request.expect("pull request info");
        assert_eq!(pr.number, 42);
        assert_eq!(pr.base_ref, "main");
        assert_eq!(pr.head_ref, "feature-x");
        assert_eq!(pr.head_sha.as_deref(), Some(PR_HEAD_SHA));
    }

    /// A job that runs `git checkout <base sha>` mid-run: this run measures
    /// the base itself, so it has no baseline of its own.
    #[test]
    fn base_checkout_carries_no_base_sha() {
        let head = head_sha();
        let event_path = write_event_payload("base-checkout", &head);
        let meta = run_example(&[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", OTHER_SHA),
            ("GITHUB_REF", "refs/pull/42/merge"),
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("GITHUB_EVENT_PATH", &event_path.to_string_lossy()),
            ("GITHUB_BASE_REF", "main"),
            ("GITHUB_HEAD_REF", "feature-x"),
        ]);
        let _ = std::fs::remove_file(&event_path);

        let git = meta.git.expect("git info");
        assert_eq!(git.sha, head);
        assert_eq!(git.base_sha, None);

        // The run is still a pull request run whatever it checked out.
        let ci = meta.ci.expect("ci info");
        assert_eq!(ci.event, "pull_request");
        assert_eq!(ci.pull_request.expect("pull request info").base_ref, "main");
    }

    /// The number comes from `GITHUB_REF`, so an unreadable event file costs
    /// only `base_sha`. Grouping the branch names with the number must not let
    /// one bad payload take out `base_ref`, the server's fallback for exactly
    /// the sha that payload was carrying.
    #[test]
    fn unreadable_event_file_keeps_the_pull_request_branches() {
        let head = head_sha();
        let meta = run_example(&[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", &head),
            ("GITHUB_REF", "refs/pull/42/merge"),
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("GITHUB_EVENT_PATH", "/nonexistent/event.json"),
            ("GITHUB_BASE_REF", "main"),
            ("GITHUB_HEAD_REF", "feature-x"),
        ]);

        assert_eq!(meta.git.expect("git info").base_sha, None);

        let ci = meta.ci.expect("ci info");
        let pr = ci.pull_request.expect("pull request info");
        assert_eq!(pr.number, 42);
        assert_eq!(pr.base_ref, "main");
        assert_eq!(pr.head_ref, "feature-x");
        assert_eq!(pr.head_sha, None);
    }

    /// `head_sha` is the one field in the group with no environment fallback,
    /// so a payload that omits the head must still leave the rest of it.
    #[test]
    fn event_payload_without_head_keeps_the_rest() {
        let head = head_sha();
        let event_path = write_event("no-head", BASE_SHA, None);
        let meta = run_example(&[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", &head),
            ("GITHUB_REF", "refs/pull/42/merge"),
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("GITHUB_EVENT_PATH", &event_path.to_string_lossy()),
            ("GITHUB_BASE_REF", "main"),
            ("GITHUB_HEAD_REF", "feature-x"),
        ]);
        let _ = std::fs::remove_file(&event_path);

        assert_eq!(
            meta.git.expect("git info").base_sha.as_deref(),
            Some(BASE_SHA)
        );
        let pr = meta
            .ci
            .expect("ci info")
            .pull_request
            .expect("pull request info");
        assert_eq!(pr.number, 42);
        assert_eq!(pr.base_ref, "main");
        assert_eq!(pr.head_ref, "feature-x");
        assert_eq!(pr.head_sha, None);
    }

    #[test]
    fn push_runs_carry_no_base_sha() {
        let head = head_sha();
        let meta = run_example(&[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_SHA", &head),
            ("GITHUB_REF", "refs/heads/main"),
            ("GITHUB_EVENT_NAME", "push"),
            ("GITHUB_EVENT_PATH", "/nonexistent/event.json"),
        ]);

        let git = meta.git.expect("git info");
        assert_eq!(git.sha, head);
        assert_eq!(git.r#ref.as_deref(), Some("refs/heads/main"));
        assert_eq!(git.base_sha, None);

        let ci = meta.ci.expect("ci info");
        assert_eq!(ci.event, "push");
        assert!(ci.pull_request.is_none());
    }

    const SHARED_POLICY: &str = "# shared\n[functions.timing]\nmin_percent_change = 5\n";
    const CI_POLICY: &str = "# ci\n[functions.alloc]\nmin_percent_change = 10\n";
    const SPECIAL_POLICY: &str = "# special\n[sql]\n";

    /// A directory that stands for a checkout: it has a `.git` and whatever
    /// files a test writes. Removed on drop, so a failed test leaves nothing.
    struct Checkout {
        root: PathBuf,
    }

    impl Checkout {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hotpath-cloud-meta-policy-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join(".git")).unwrap();
            Self { root }
        }

        fn write(&self, path: &str, contents: &[u8]) -> &Self {
            let path = self.root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
            self
        }

        /// Runs the example from the checkout. `HOTPATH_SOURCE_ROOT` makes
        /// the git root above the working directory the report's checkout.
        fn run(&self, envs: &[(&str, &str)]) -> (JsonMeta, String) {
            let mut all = vec![("HOTPATH_SOURCE_ROOT", "")];
            all.extend_from_slice(envs);
            run_example_in(Some(&self.root), &all)
        }
    }

    /// A directory outside any repository.
    fn outside_any_repository(name: &str) -> Checkout {
        let checkout = Checkout::new(name);
        std::fs::remove_dir(checkout.root.join(".git")).unwrap();
        checkout
    }

    impl Drop for Checkout {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn policy_of_the_benchmark_wins_over_the_shared_one() {
        let checkout = Checkout::new("benchmark-file");
        checkout
            .write("hotpath/policy.toml", SHARED_POLICY.as_bytes())
            .write("hotpath/ci-policy.toml", CI_POLICY.as_bytes());

        let (meta, stderr) = checkout.run(&[("HOTPATH_BENCHMARK", "ci")]);
        let policy = meta.policy.expect("policy");
        assert_eq!(policy.path, "hotpath/ci-policy.toml");
        // As written, comments included.
        assert_eq!(policy.source, CI_POLICY);
        assert!(!stderr.contains("policy file"), "{stderr}");

        // A benchmark without its own file uses the shared one.
        let (meta, _) = checkout.run(&[("HOTPATH_BENCHMARK", "nightly")]);
        let policy = meta.policy.expect("policy");
        assert_eq!(policy.path, "hotpath/policy.toml");
        assert_eq!(policy.source, SHARED_POLICY);
    }

    #[test]
    fn checkout_without_a_policy_file_sends_none() {
        let checkout = Checkout::new("no-file");
        checkout.write("hotpath/notes.toml", b"x");

        // A local run: the report is written and nothing is said.
        let (meta, stderr) = checkout.run(&[("HOTPATH_BENCHMARK", "ci")]);
        assert_eq!(meta.policy, None);
        assert!(!stderr.contains("policy file"), "{stderr}");

        // The benchmark job of a relay writes the report and does not upload
        // it: its log says why the relay will be refused.
        let (meta, stderr) = checkout.run(&[
            ("HOTPATH_BENCHMARK", "ci"),
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_EVENT_NAME", "pull_request"),
        ]);
        assert_eq!(meta.policy, None);
        assert!(
            stderr.contains(
                "hotpath: the report carries no policy file and hotpath.rs refuses reports \
                 without one. Add `hotpath/ci-policy.toml` or `hotpath/policy.toml`"
            ),
            "{stderr}"
        );
    }

    #[test]
    fn policy_path_override_wins_over_the_policy_directory() {
        let checkout = Checkout::new("override");
        checkout
            .write("hotpath/policy.toml", SHARED_POLICY.as_bytes())
            .write("hotpath/ci-policy.toml", CI_POLICY.as_bytes())
            .write("config/special.toml", SPECIAL_POLICY.as_bytes());

        // Relative to the working directory.
        let (meta, _) = checkout.run(&[
            ("HOTPATH_BENCHMARK", "ci"),
            ("HOTPATH_POLICY_PATH", "config/special.toml"),
        ]);
        let policy = meta.policy.expect("policy");
        assert_eq!(policy.path, "config/special.toml");
        assert_eq!(policy.source, SPECIAL_POLICY);
    }

    #[test]
    fn unusable_policy_file_is_reported_and_not_sent() {
        let checkout = Checkout::new("unusable");
        checkout
            .write("hotpath/policy.toml", SHARED_POLICY.as_bytes())
            .write("hotpath/blank-policy.toml", b" \n\t\n")
            .write("hotpath/large-policy.toml", &vec![b'#'; 65537])
            .write("hotpath/latin-policy.toml", b"name = \"\xff\xfe\"\n");

        // Never treated as "no file": the shared file does not stand in.
        for (benchmark, message) in [
            (
                "blank",
                "hotpath: the policy file `hotpath/blank-policy.toml` is blank.",
            ),
            (
                "large",
                "hotpath: the policy file `hotpath/large-policy.toml` is larger than 65536 bytes",
            ),
            (
                "latin",
                "hotpath: the policy file `hotpath/latin-policy.toml` is not valid UTF-8",
            ),
        ] {
            let (meta, stderr) = checkout.run(&[("HOTPATH_BENCHMARK", benchmark)]);
            assert_eq!(meta.policy, None, "{benchmark}");
            assert!(stderr.contains(message), "{benchmark}: {stderr}");
            assert!(
                stderr.contains("The report carries no policy."),
                "{benchmark}: {stderr}"
            );
        }
    }

    #[test]
    fn policy_path_override_outside_the_repository_is_refused() {
        let checkout = Checkout::new("override-outside");
        checkout.write("hotpath/policy.toml", SHARED_POLICY.as_bytes());
        let outside = std::env::temp_dir().join(format!(
            "hotpath-cloud-meta-policy-{}-outside.toml",
            std::process::id()
        ));
        std::fs::write(&outside, SPECIAL_POLICY).unwrap();

        let (meta, stderr) = checkout.run(&[("HOTPATH_POLICY_PATH", outside.to_str().unwrap())]);
        let _ = std::fs::remove_file(&outside);

        assert_eq!(meta.policy, None);
        assert!(stderr.contains("is outside the repository"), "{stderr}");
        assert!(stderr.contains("(HOTPATH_POLICY_PATH)"), "{stderr}");
    }

    /// Without `HOTPATH_SOURCE_ROOT` the checkout is verified against the
    /// registered source locations. A report that cannot be verified still
    /// carries the policy of the repository it ran in.
    #[test]
    fn policy_is_found_when_the_checkout_cannot_be_verified() {
        let checkout = Checkout::new("unverified");
        checkout
            .write("hotpath/policy.toml", SHARED_POLICY.as_bytes())
            .write("hotpath/ci-policy.toml", CI_POLICY.as_bytes());

        for example in [
            // Registers no source location at all.
            "no_locations",
            // Registers locations that no ancestor of the working directory has.
            "basic_all_features",
        ] {
            let (meta, stderr) = run_named_example_in(
                example,
                Some(&checkout.root),
                &[("HOTPATH_BENCHMARK", "ci")],
            );
            let policy = meta
                .policy
                .unwrap_or_else(|| panic!("{example}: no policy\n{stderr}"));
            assert_eq!(policy.path, "hotpath/ci-policy.toml", "{example}");
            assert_eq!(policy.source, CI_POLICY, "{example}");
            // The commit identity keeps its stricter rule.
            assert!(meta.git.is_none(), "{example}: {:?}", meta.git);
        }
    }

    #[test]
    fn upload_outside_any_repository_says_it_has_no_policy() {
        let outside = outside_any_repository("no-repository");
        outside.write("hotpath/policy.toml", SHARED_POLICY.as_bytes());
        let message = "hotpath: no git repository found from the working directory";

        let (meta, stderr) = run_named_example_in(
            "no_locations",
            Some(&outside.root),
            &[("HOTPATH_UPLOAD", "1")],
        );
        assert_eq!(meta.policy, None);
        assert!(stderr.contains(message), "{stderr}");

        // Not an upload: nothing to warn about.
        let (meta, stderr) = run_named_example_in("no_locations", Some(&outside.root), &[]);
        assert_eq!(meta.policy, None);
        assert!(!stderr.contains(message), "{stderr}");
    }
}
