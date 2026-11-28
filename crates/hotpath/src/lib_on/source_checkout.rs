//! Locates the source checkout a report's relative `location.file` values
//! were compiled from: the build workspace root (`meta.source_root`, relative
//! to the git root) and the git root the commit identity and policy are read
//! from. Every value is verified against the filesystem; when nothing
//! verifies, the source root is omitted rather than guessed.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::json::policy_file::find_git_root;

/// `HOTPATH_SOURCE_ROOT`: the source root as given, unverified.
static SOURCE_ROOT_OVERRIDE: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("HOTPATH_SOURCE_ROOT").ok());

/// Where `#[hotpath::main]` was compiled: the user package's
/// `CARGO_MANIFEST_DIR` and the main fn's `file!()`, which cargo makes
/// relative to the build workspace root. Both are compile-time constants, so
/// the native and the Callgrind run of one binary resolve identically.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SourceAnchor {
    pub(crate) manifest_dir: &'static str,
    pub(crate) main_file: &'static str,
}

/// The resolved checkout. `git_root` is `None` when the checkout could not be
/// verified; the caller then reads git metadata and the policy from the git
/// root around the working directory.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SourceCheckout {
    pub(crate) source_root: Option<String>,
    pub(crate) git_root: Option<PathBuf>,
}

impl SourceCheckout {
    const UNRESOLVED: Self = Self {
        source_root: None,
        git_root: None,
    };
}

/// Resolves the checkout once per report.
///
/// `HOTPATH_SOURCE_ROOT` wins and makes the git root around the working
/// directory the checkout. Otherwise see [`resolve_at`].
pub(crate) fn resolve(anchor: Option<SourceAnchor>) -> SourceCheckout {
    let cwd = std::env::current_dir().ok();
    if let Some(source_root) = SOURCE_ROOT_OVERRIDE.as_ref() {
        return SourceCheckout {
            source_root: Some(source_root.clone()),
            git_root: cwd.as_deref().and_then(find_git_root),
        };
    }
    match cwd {
        Some(cwd) => resolve_at(&cwd, anchor, crate::lib_on::locations::any_relative_file),
        None => SourceCheckout::UNRESOLVED,
    }
}

/// With an anchor, the build workspace is derived from it when it lies in
/// the checkout around `cwd` (a binary run where it was built). When it does
/// not (a copied binary whose build directory is gone, or one built in
/// another checkout) the anchor's main file is looked up from `cwd` upwards.
/// An ambiguous anchor or one whose main file is not a workspace-relative
/// path resolves nothing: any pick could be the wrong root.
///
/// Without an anchor (builder API, Criterion) the lookup from `cwd` upwards
/// probes one registered relative file, the same one in every run that
/// registered it.
fn resolve_at(
    cwd: &Path,
    anchor: Option<SourceAnchor>,
    registered_file: impl FnOnce() -> Option<&'static str>,
) -> SourceCheckout {
    let Some(anchor) = anchor else {
        return registered_file().map_or(SourceCheckout::UNRESOLVED, |probe| {
            walk_up(cwd, Path::new(probe))
        });
    };
    let Some(main_file) = workspace_relative(anchor.main_file) else {
        return SourceCheckout::UNRESOLVED;
    };
    match anchored_workspace(Path::new(anchor.manifest_dir), &main_file, cwd) {
        Anchored::Resolved(checkout) => checkout,
        Anchored::Ambiguous => SourceCheckout::UNRESOLVED,
        Anchored::Unavailable => walk_up(cwd, &main_file),
    }
}

enum Anchored {
    Resolved(SourceCheckout),
    /// No candidate in the checkout around the working directory.
    Unavailable,
    /// More than one candidate in that checkout.
    Ambiguous,
}

/// The build workspace root `W` is the ancestor of the package directory
/// that splits `main_file` into "package directory relative to `W`" plus
/// "path inside the package", with the file present. Only candidates whose
/// git root is the working directory's count, so the report never describes
/// a different checkout than the one it runs in.
fn anchored_workspace(manifest_dir: &Path, main_file: &Path, cwd: &Path) -> Anchored {
    let Some(cwd_git_root) = find_git_root(cwd) else {
        return Anchored::Unavailable;
    };
    let Ok(canonical_git_root) = cwd_git_root.canonicalize() else {
        return Anchored::Unavailable;
    };
    let mut candidates = manifest_dir.ancestors().filter(|workspace| {
        manifest_dir
            .strip_prefix(workspace)
            .is_ok_and(|package| main_file.starts_with(package))
            && workspace.join(main_file).is_file()
            && find_git_root(workspace)
                .and_then(|root| root.canonicalize().ok())
                .is_some_and(|root| root == canonical_git_root)
    });
    let Some(workspace) = candidates.next() else {
        return Anchored::Unavailable;
    };
    if candidates.next().is_some() {
        return Anchored::Ambiguous;
    }
    let source_root = workspace
        .canonicalize()
        .ok()
        .and_then(|workspace| relative_slash_path(&workspace, &canonical_git_root));
    match source_root {
        Some(source_root) => Anchored::Resolved(SourceCheckout {
            source_root: Some(source_root),
            git_root: Some(cwd_git_root),
        }),
        None => Anchored::Unavailable,
    }
}

/// The nearest ancestor of `cwd` containing `probe` is the build workspace,
/// and its git root the checkout.
fn walk_up(cwd: &Path, probe: &Path) -> SourceCheckout {
    let Some(workspace) = cwd.ancestors().find(|dir| dir.join(probe).is_file()) else {
        return SourceCheckout::UNRESOLVED;
    };
    let Some(git_root) = find_git_root(workspace) else {
        return SourceCheckout::UNRESOLVED;
    };
    SourceCheckout {
        source_root: relative_slash_path(workspace, &git_root),
        git_root: Some(git_root),
    }
}

/// `file!()` as a path relative to the build workspace, or `None` when it is
/// absolute or leaves the workspace (generated sources, `#[path]` or
/// `[[bin]] path` outside it): such a path cannot anchor anything, and
/// joined onto a directory an absolute one would match anywhere.
fn workspace_relative(file: &str) -> Option<PathBuf> {
    if crate::lib_on::locations::is_absolute_path(file) {
        return None;
    }
    let mut path = PathBuf::new();
    for segment in file.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                if !path.pop() {
                    return None;
                }
            }
            _ => path.push(segment),
        }
    }
    (!path.as_os_str().is_empty()).then_some(path)
}

fn relative_slash_path(path: &Path, base: &Path) -> Option<String> {
    let relative = path.strip_prefix(base).ok()?;
    Some(relative.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::lib_on::source_checkout::{resolve_at, SourceAnchor, SourceCheckout};

    /// A temp directory tree, removed on drop. `.git` marks a git root.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "hotpath-source-checkout-{}-{name}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root.canonicalize().unwrap())
        }

        fn with(self, paths: &[&str]) -> Self {
            for path in paths {
                let path = self.0.join(path);
                if path.ends_with(".git") {
                    std::fs::create_dir_all(path).unwrap();
                } else {
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, "").unwrap();
                }
            }
            self
        }

        fn path(&self, path: &str) -> PathBuf {
            self.0.join(path)
        }

        /// An anchor whose package directory is `package` inside this tree.
        fn anchor(&self, package: &str, main_file: &'static str) -> SourceAnchor {
            let manifest_dir = self.path(package).to_str().unwrap().to_string();
            SourceAnchor {
                manifest_dir: Box::leak(manifest_dir.into_boxed_str()),
                main_file,
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn resolved(source_root: &str, git_root: &Path) -> SourceCheckout {
        SourceCheckout {
            source_root: Some(source_root.to_string()),
            git_root: Some(git_root.to_path_buf()),
        }
    }

    fn no_registry() -> Option<&'static str> {
        None
    }

    #[test]
    fn anchor_resolves_the_workspace_root_not_the_package_dir() {
        let repo = Tree::new("member").with(&[".git", "crates/app/src/main.rs"]);
        let anchor = repo.anchor("crates/app", "crates/app/src/main.rs");
        assert_eq!(
            resolve_at(&repo.0, Some(anchor), no_registry),
            resolved("", &repo.0)
        );

        let anchor = repo.anchor("crates/app", "src/main.rs");
        // `crates/app/src/main.rs` as the root package of a workspace in
        // `crates/app`: launched from above, the anchor still finds it.
        assert_eq!(
            resolve_at(&repo.0, Some(anchor), no_registry),
            resolved("crates/app", &repo.0)
        );
    }

    /// A root crate and a nested workspace that share only `src/main.rs`:
    /// launched from the repository root, the anchor names the nested
    /// workspace, while the registry probe alone accepts the root (the
    /// documented builder-API limitation).
    #[test]
    fn anchor_separates_a_nested_workspace_sharing_the_main_file() {
        let repo = Tree::new("shared-main").with(&[".git", "src/main.rs", "bench/src/main.rs"]);
        let anchor = repo.anchor("bench", "src/main.rs");
        assert_eq!(
            resolve_at(&repo.0, Some(anchor), || Some("src/main.rs")),
            resolved("bench", &repo.0)
        );
        assert_eq!(
            resolve_at(&repo.0, None, || Some("src/main.rs")),
            resolved("", &repo.0)
        );
    }

    /// The anchored result never reads the registry, so runs that
    /// registered different locations agree.
    #[test]
    fn anchored_resolution_ignores_registered_locations() {
        let repo = Tree::new("registry").with(&[".git", "bench/src/main.rs", "bench/src/lib.rs"]);
        let anchor = repo.anchor("bench", "src/main.rs");
        for registered in [None, Some("src/lib.rs"), Some("src/other.rs")] {
            assert_eq!(
                resolve_at(&repo.0, Some(anchor), || registered),
                resolved("bench", &repo.0),
                "{registered:?}"
            );
        }
    }

    /// Two candidates in the runtime checkout resolve nothing, even though a
    /// lookup from the working directory would match.
    #[test]
    fn ambiguous_anchor_omits_the_source_root() {
        let repo = Tree::new("ambiguous").with(&[".git", "x/x/src/main.rs", "x/x/x/src/main.rs"]);
        let anchor = repo.anchor("x/x", "x/src/main.rs");
        let cwd = repo.path("x");
        assert_eq!(
            resolve_at(&cwd, Some(anchor), no_registry),
            SourceCheckout::UNRESOLVED
        );
    }

    /// Built in checkout A, run in checkout B: B's own files and git root
    /// describe the report, never A's.
    #[test]
    fn binary_built_in_another_checkout_resolves_in_the_runtime_one() {
        let built = Tree::new("checkout-a").with(&[".git", "bench/src/main.rs"]);
        let run = Tree::new("checkout-b").with(&[".git", "bench/src/main.rs"]);
        let anchor = built.anchor("bench", "src/main.rs");

        let inside = run.path("bench");
        assert_eq!(
            resolve_at(&inside, Some(anchor), no_registry),
            resolved("bench", &run.0)
        );
        // From above B's workspace the lookup cannot reach it.
        assert_eq!(
            resolve_at(&run.0, Some(anchor), no_registry),
            SourceCheckout::UNRESOLVED
        );
    }

    /// A copied binary whose build directory no longer exists falls back to
    /// the lookup from the working directory.
    #[test]
    fn deleted_build_directory_falls_back_to_the_working_directory() {
        let run = Tree::new("copied").with(&[".git", "bench/src/main.rs"]);
        let gone = SourceAnchor {
            manifest_dir: "/nonexistent/hotpath-build/bench",
            main_file: "src/main.rs",
        };
        assert_eq!(
            resolve_at(&run.path("bench/src"), Some(gone), no_registry),
            resolved("bench", &run.0)
        );
        assert_eq!(
            resolve_at(&run.0, Some(gone), no_registry),
            SourceCheckout::UNRESOLVED
        );
    }

    /// A main file that is absolute or leaves the workspace cannot anchor or
    /// probe anything: no source root, and never the working directory
    /// itself, even with registered locations available.
    #[test]
    fn unusable_main_file_omits_the_source_root() {
        let repo = Tree::new("unusable").with(&[".git", "src/main.rs"]);
        let absolute = repo.path("src/main.rs").to_str().unwrap().to_string();
        let absolute: &'static str = Box::leak(absolute.into_boxed_str());
        for main_file in [absolute, "../shared/main.rs", "C:/build/src/main.rs", ""] {
            let anchor = SourceAnchor {
                manifest_dir: repo.anchor("", "").manifest_dir,
                main_file,
            };
            assert_eq!(
                resolve_at(&repo.0, Some(anchor), || Some("src/main.rs")),
                SourceCheckout::UNRESOLVED,
                "{main_file}"
            );
        }
    }

    /// A symlinked path to the checkout still passes the same-checkout check.
    #[cfg(unix)]
    #[test]
    fn symlinked_checkout_passes_the_same_checkout_check() {
        let repo = Tree::new("symlinked").with(&[".git", "bench/src/main.rs"]);
        let link = Tree::new("symlinked-link");
        let linked = link.path("repo");
        std::os::unix::fs::symlink(&repo.0, &linked).unwrap();
        let anchor = repo.anchor("bench", "src/main.rs");
        let checkout = resolve_at(&linked, Some(anchor), no_registry);
        assert_eq!(checkout.source_root.as_deref(), Some("bench"));
        assert_eq!(checkout.git_root, Some(linked));
    }

    #[test]
    fn no_verifiable_root_resolves_nothing() {
        let outside = Tree::new("outside").with(&["src/main.rs"]);
        let anchor = outside.anchor("", "src/main.rs");
        assert_eq!(
            resolve_at(&outside.0, Some(anchor), no_registry),
            SourceCheckout::UNRESOLVED
        );
        assert_eq!(
            resolve_at(&outside.0, None, || Some("src/main.rs")),
            SourceCheckout::UNRESOLVED
        );
        assert_eq!(
            resolve_at(&outside.0, None, no_registry),
            SourceCheckout::UNRESOLVED
        );
    }
}
