//! Reads `sha`, `ref` and `repository` for the report `meta.git` object
//! straight from the `.git` directory. Only `committed_at` shells out to git,
//! since it needs the commit object, which may be packed and is always
//! compressed. In CI the environment supplies the ref fields more precisely
//! (see `ci_info`); this reader serves local repo-token uploads and offline
//! reports.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

pub(crate) fn read_git_info_at(root: &Path) -> Option<crate::json::JsonGitInfo> {
    let git_dir = resolve_git_dir(root)?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    let (sha, ref_name) = if let Some(ref_name) = head.strip_prefix("ref: ") {
        let ref_name = ref_name.trim().to_string();
        (resolve_ref(&git_dir, &ref_name)?, Some(ref_name))
    } else if is_sha(head) {
        // Detached HEAD holds the commit directly.
        (head.to_string(), None)
    } else {
        return None;
    };
    Some(crate::json::JsonGitInfo {
        r#ref: ref_name,
        // TODO: a merge base against the default branch needs a commit-graph
        // walk this pure `.git` reader deliberately does not do.
        base_sha: None,
        repository: origin_repository(&git_dir),
        committed_at: committed_at(root, &sha),
        sha,
    })
}

/// Committer (not author) timestamp of `sha`, in UTC. `None` when git is
/// missing, refuses the repository (e.g. `safe.directory`), lacks the object
/// or prints something unparsable - the report is written without it.
fn committed_at(root: &Path, sha: &str) -> Option<OffsetDateTime> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", "--no-patch", "--no-show-signature", "--format=%cI"])
        .arg(sha)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = std::str::from_utf8(&output.stdout).ok()?;
    let committed = OffsetDateTime::parse(stdout.trim(), &Rfc3339).ok()?;
    Some(committed.to_offset(UtcOffset::UTC))
}

/// "owner/name" from the `origin` remote's URL, for both the SSH
/// (`git@host:owner/name.git`) and HTTPS (`https://host/owner/name.git`)
/// forms. Informational only - the server authorizes from the upload
/// credential.
fn origin_repository(git_dir: &Path) -> Option<String> {
    let config = std::fs::read_to_string(common_dir(git_dir).join("config")).ok()?;
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // Git matches section names case-insensitively, subsections not.
            let section = line.replace(char::is_whitespace, "");
            in_origin = section
                .strip_suffix("\"origin\"]")
                .is_some_and(|name| name.eq_ignore_ascii_case("[remote"));
        } else if in_origin {
            if let Some((key, url)) = line.split_once('=') {
                if key.trim() == "url" {
                    return owner_name(url.trim());
                }
            }
        }
    }
    None
}

fn owner_name(url: &str) -> Option<String> {
    let url = url.trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    // Only a remote with a host names an owner; a filesystem remote
    // ("/srv/repos/foo") would otherwise yield a plausible-looking
    // "repos/foo".
    let path = match url.split_once("://") {
        Some(("file", _)) => return None,
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':').filter(|(host, _)| !host.is_empty())?.1,
    };
    let mut segments = path.rsplit('/');
    let name = segments.next().filter(|s| !s.is_empty())?;
    let owner = segments.next().filter(|s| !s.is_empty())?;
    Some(format!("{owner}/{name}"))
}

/// `.git` is a plain directory for regular checkouts; worktrees and
/// submodules use a file with a `gitdir: <path>` line pointing at the real
/// directory.
fn resolve_git_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let contents = std::fs::read_to_string(&dot_git).ok()?;
    let gitdir = contents.strip_prefix("gitdir:")?.trim();
    let path = Path::new(gitdir);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    })
}

/// Loose ref first (in the git dir, then the common dir shared between
/// worktrees), falling back to `packed-refs`.
fn resolve_ref(git_dir: &Path, ref_name: &str) -> Option<String> {
    let common_dir = common_dir(git_dir);

    for dir in [git_dir, common_dir.as_path()] {
        if let Ok(contents) = std::fs::read_to_string(dir.join(ref_name)) {
            let sha = contents.trim();
            if is_sha(sha) {
                return Some(sha.to_string());
            }
        }
    }

    let packed = std::fs::read_to_string(common_dir.join("packed-refs")).ok()?;
    for line in packed.lines() {
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        if let Some((sha, name)) = line.split_once(' ') {
            if name.trim() == ref_name && is_sha(sha.trim()) {
                return Some(sha.trim().to_string());
            }
        }
    }
    None
}

/// A worktree's git dir holds only its own HEAD; refs, `packed-refs` and
/// `config` live in the common dir it points at.
fn common_dir(git_dir: &Path) -> PathBuf {
    match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(rel) => {
            let rel = rel.trim();
            let path = Path::new(rel);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                git_dir.join(rel)
            }
        }
        Err(_) => git_dir.to_path_buf(),
    }
}

fn is_sha(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use crate::lib_on::git_info::{owner_name, read_git_info_at};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use time::macros::datetime;

    fn temp_repo(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hotpath-git-info-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SHA: &str = "4c36978500000000000000000000000000000000";

    #[test]
    fn reads_loose_ref() {
        let root = temp_repo("loose");
        let git = root.join(".git");
        std::fs::create_dir_all(git.join("refs/heads")).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(git.join("refs/heads/main"), format!("{SHA}\n")).unwrap();

        let info = read_git_info_at(&root).unwrap();
        assert_eq!(info.sha, SHA);
        assert_eq!(info.r#ref.as_deref(), Some("refs/heads/main"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_packed_ref() {
        let root = temp_repo("packed");
        let git = root.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/develop\n").unwrap();
        std::fs::write(
            git.join("packed-refs"),
            format!("# pack-refs with: peeled fully-peeled sorted\n{SHA} refs/heads/develop\n"),
        )
        .unwrap();

        let info = read_git_info_at(&root).unwrap();
        assert_eq!(info.sha, SHA);
        assert_eq!(info.r#ref.as_deref(), Some("refs/heads/develop"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_detached_head() {
        let root = temp_repo("detached");
        let git = root.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), format!("{SHA}\n")).unwrap();

        let info = read_git_info_at(&root).unwrap();
        assert_eq!(info.sha, SHA);
        assert_eq!(info.r#ref, None);
        // No commit object to read: the field is omitted, the info is not.
        assert_eq!(info.committed_at, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Runs git isolated from the user's config (signing, hooks, identity).
    fn git(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=hotpath",
                "-c",
                "user.email=hotpath@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .envs(envs.iter().copied())
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn commit(dir: &Path, author_date: &str, committer_date: &str) -> String {
        git(
            dir,
            &["commit", "--allow-empty", "--no-verify", "-m", "c"],
            &[
                ("GIT_AUTHOR_DATE", author_date),
                ("GIT_COMMITTER_DATE", committer_date),
            ],
        );
        git(dir, &["rev-parse", "HEAD"], &[])
    }

    /// Committer date, not author date, of the checked-out commit - also
    /// when HEAD is detached at an older commit - normalized to UTC.
    #[test]
    fn reads_committer_timestamp_of_checked_out_commit() {
        let root = temp_repo("committed-at");
        git(&root, &["init", "-q", "-b", "main"], &[]);
        let first = commit(
            &root,
            "2020-01-01T00:00:00+00:00",
            "2026-11-15T15:30:00+01:00",
        );
        let second = commit(
            &root,
            "2020-01-02T00:00:00+00:00",
            "2026-11-16T09:00:00+00:00",
        );

        let info = read_git_info_at(&root).unwrap();
        assert_eq!(info.sha, second);
        assert_eq!(info.r#ref.as_deref(), Some("refs/heads/main"));
        assert_eq!(info.committed_at, Some(datetime!(2026-11-16 09:00:00 UTC)));

        git(&root, &["checkout", "-q", "--detach", &first], &[]);
        let info = read_git_info_at(&root).unwrap();
        assert_eq!(info.sha, first);
        assert_eq!(info.r#ref, None);
        let committed_at = info.committed_at.unwrap();
        assert_eq!(committed_at, datetime!(2026-11-15 14:30:00 UTC));
        assert!(committed_at.offset().is_utc());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_origin_repository() {
        let root = temp_repo("origin");
        let git = root.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), format!("{SHA}\n")).unwrap();
        std::fs::write(
            git.join("config"),
            "[core]\n\turl = not-a-remote\n[remote \"upstream\"]\n\turl = git@github.com:other/repo.git\n[remote \"origin\"]\n\turl = git@github.com:hotpath-rs/hotpath.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n",
        )
        .unwrap();

        let info = read_git_info_at(&root).unwrap();
        assert_eq!(info.repository.as_deref(), Some("hotpath-rs/hotpath"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn origin_repository_absent_without_config() {
        let root = temp_repo("no-config");
        let git = root.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), format!("{SHA}\n")).unwrap();

        let info = read_git_info_at(&root).unwrap();
        assert_eq!(info.repository, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn owner_name_url_forms() {
        for url in [
            "git@github.com:hotpath-rs/hotpath.git",
            "https://github.com/hotpath-rs/hotpath.git",
            "https://github.com/hotpath-rs/hotpath",
            "ssh://git@github.com/hotpath-rs/hotpath.git",
            "https://github.com/hotpath-rs/hotpath/",
        ] {
            assert_eq!(
                owner_name(url).as_deref(),
                Some("hotpath-rs/hotpath"),
                "{url}"
            );
        }
        for url in [
            "hotpath-rs",
            "",
            "/srv/repos/hotpath-rs",
            "../repos/hotpath-rs",
            "file:///srv/repos/hotpath-rs",
            "https://github.com/pawurb",
        ] {
            assert_eq!(owner_name(url), None, "{url}");
        }
    }

    #[test]
    fn reads_worktree_gitdir_file() {
        let root = temp_repo("worktree");
        let real = root.join("real-git");
        std::fs::create_dir_all(real.join("worktrees/wt")).unwrap();
        std::fs::create_dir_all(real.join("refs/heads")).unwrap();
        let checkout = root.join("checkout");
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", real.join("worktrees/wt").display()),
        )
        .unwrap();
        std::fs::write(real.join("worktrees/wt/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(
            real.join("worktrees/wt/commondir"),
            format!("{}\n", real.display()),
        )
        .unwrap();
        std::fs::write(real.join("refs/heads/main"), format!("{SHA}\n")).unwrap();

        let info = read_git_info_at(&checkout).unwrap();
        assert_eq!(info.sha, SHA);
        assert_eq!(info.r#ref.as_deref(), Some("refs/heads/main"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
