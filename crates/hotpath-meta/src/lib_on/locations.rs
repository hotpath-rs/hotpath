//! Call-site location registry. Instrumentation macros register a static
//! [`Location`] under the same identity string used for stats aggregation
//! (function name, resource id string, debug loc); report builders join on
//! that string at build time, so the hot path never carries location data.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::json::JsonLocation;

/// Structured source location of an instrumented call site, captured at
/// compile time by the instrumentation macros.
pub struct Location {
    pub file: &'static str,
    pub line: u32,
    pub column: u32,
}

static LOCATIONS: OnceLock<RwLock<HashMap<&'static str, &'static Location>>> = OnceLock::new();

/// Registers `location` under `name`. First registration wins: two call sites
/// sharing one identity string (e.g. two `#[measure(label = "x")]` sites)
/// already merge into a single stats entry, and its location is whichever
/// site registered first.
#[doc(hidden)]
pub fn register_location(name: &'static str, location: &'static Location) {
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let map = LOCATIONS.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(mut w) = map.write() {
        w.entry(name).or_insert(location);
    }
}

/// Registers a location captured at runtime by the deprecated
/// `wrap::*::new()` constructors (`#[track_caller]`). The `Location` is
/// heap-allocated and leaked, but only for a key not registered yet, so
/// repeat constructions at one call site do not accumulate.
pub(crate) fn register_caller_location(
    key: &'static str,
    caller: &'static std::panic::Location<'static>,
) {
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let map = LOCATIONS.get_or_init(|| RwLock::new(HashMap::new()));
    if let Ok(mut w) = map.write() {
        w.entry(key).or_insert_with(|| {
            &*Box::leak(Box::new(Location {
                file: caller.file(),
                line: caller.line(),
                column: caller.column(),
            }))
        });
    }
}

/// Joins an entry's identity string against the registry, normalizing the
/// file path for shipping in a report.
pub(crate) fn lookup_location(name: &str) -> Option<JsonLocation> {
    let map = LOCATIONS.get()?;
    let location = map.read().ok()?.get(name).copied()?;
    Some(JsonLocation {
        file: normalize_file_path(location.file),
        line: location.line,
        column: location.column,
    })
}

/// Location for a resource entry, whose `key` is a `file:line:column`
/// call-site id that always has a registration. The parse fallback keeps
/// this total if the registry is ever unavailable (poisoned lock).
pub(crate) fn location_for_key(key: &str) -> JsonLocation {
    lookup_location(key).unwrap_or_else(|| {
        let mut parts = key.rsplitn(3, ':');
        match (
            parts.next().and_then(|c| c.parse().ok()),
            parts.next().and_then(|l| l.parse().ok()),
            parts.next(),
        ) {
            (Some(column), Some(line), Some(file)) => JsonLocation {
                file: normalize_file_path(file),
                line,
                column,
            },
            _ => JsonLocation {
                file: key.to_string(),
                line: 0,
                column: 0,
            },
        }
    })
}

/// First registered file path that is workspace-relative; used to locate the
/// build workspace root at report time (`report_meta::source_root`). Any
/// relative file works: they are all relative to the same workspace root.
pub(crate) fn any_relative_file() -> Option<&'static str> {
    let map = LOCATIONS.get()?;
    let guard = map.read().ok()?;
    guard
        .values()
        .map(|location| location.file)
        .find(|file| !is_absolute_path(file))
}

/// `file!()` is workspace-root-relative for workspace members and absolute
/// for registry dependencies. Relative paths get their dot segments resolved
/// (`resolve_dot_segments`); absolute paths
/// under a cargo registry checkout are rewritten to
/// `<external>/<crate>-<version>/<path>` (keeps docs.rs linking possible and
/// strips `$HOME`); other absolute paths pass through - the server refuses to
/// link them.
fn normalize_file_path(file: &str) -> String {
    if !is_absolute_path(file) {
        return resolve_dot_segments(file);
    }
    for marker in ["/registry/src/", "\\registry\\src\\"] {
        if let Some(pos) = file.find(marker) {
            let rest = &file[pos + marker.len()..];
            // rest = "<index>/<crate>-<version>/<path>"
            let separator = if marker.starts_with('/') { '/' } else { '\\' };
            if let Some((_index, crate_path)) = rest.split_once(separator) {
                if !crate_path.is_empty() {
                    return format!("<external>/{}", crate_path.replace('\\', "/"));
                }
            }
        }
    }
    file.to_string()
}

/// `file!()` keeps `.` and `..` verbatim for a module loaded through
/// `#[path = "../.."]`, and the server refuses to link such a path. Resolves
/// them lexically, joining with `/`; string work only, the filesystem is never
/// touched. A `..` climbing above the workspace root has nothing to cancel and
/// is kept, so that path stays unlinked rather than pointing at a wrong file.
fn resolve_dot_segments(file: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in file.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." if segments.last().is_some_and(|last| *last != "..") => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    segments.join("/")
}

/// String-based check instead of `Path::is_absolute`, which is
/// platform-dependent: normalization must treat a path the same way
/// regardless of the OS the report is generated on.
fn is_absolute_path(file: &str) -> bool {
    file.starts_with('/') || file.starts_with('\\') || file.as_bytes().get(1) == Some(&b':')
}

#[cfg(test)]
mod tests {
    use crate::lib_on::locations::{
        lookup_location, normalize_file_path, register_location, Location,
    };

    #[test]
    fn relative_paths_pass_through() {
        assert_eq!(
            normalize_file_path("crates/hotpath/src/lib.rs"),
            "crates/hotpath/src/lib.rs"
        );
    }

    #[test]
    fn relative_dot_segments_resolve_within_the_workspace() {
        assert_eq!(
            normalize_file_path(
                "crates/test-drain-criterion/benches/../../hotpath-drain/examples/support/lockstep.rs"
            ),
            "crates/hotpath-drain/examples/support/lockstep.rs"
        );
        assert_eq!(
            normalize_file_path("crates\\app\\benches\\..\\src\\.\\lib.rs"),
            "crates/app/src/lib.rs"
        );
        assert_eq!(normalize_file_path("./src//main.rs"), "src/main.rs");
    }

    #[test]
    fn dot_segments_above_the_workspace_root_are_kept() {
        assert_eq!(
            normalize_file_path("../sibling/src/lib.rs"),
            "../sibling/src/lib.rs"
        );
        assert_eq!(
            normalize_file_path("crates/app/../../../sibling/src/lib.rs"),
            "../sibling/src/lib.rs"
        );
        assert_eq!(normalize_file_path("../../a.rs"), "../../a.rs");
    }

    #[test]
    fn registry_paths_rewrite_to_external() {
        assert_eq!(
            normalize_file_path(
                "/home/u/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.47.1/src/lib.rs"
            ),
            "<external>/tokio-1.47.1/src/lib.rs"
        );
        assert_eq!(
            normalize_file_path(
                "C:\\Users\\u\\.cargo\\registry\\src\\index.crates.io-1949cf8c6b5b557f\\tokio-1.47.1\\src\\lib.rs"
            ),
            "<external>/tokio-1.47.1/src/lib.rs"
        );
    }

    #[test]
    fn other_absolute_paths_pass_through() {
        assert_eq!(
            normalize_file_path("/home/u/project/src/main.rs"),
            "/home/u/project/src/main.rs"
        );
        assert_eq!(
            normalize_file_path("C:\\project\\src\\main.rs"),
            "C:\\project\\src\\main.rs"
        );
    }

    #[test]
    fn first_registration_wins() {
        static FIRST: Location = Location {
            file: "src/a.rs",
            line: 1,
            column: 2,
        };
        static SECOND: Location = Location {
            file: "src/b.rs",
            line: 3,
            column: 4,
        };
        register_location("locations_test_first_wins", &FIRST);
        register_location("locations_test_first_wins", &SECOND);

        let found = lookup_location("locations_test_first_wins").unwrap();
        assert_eq!(found.file, "src/a.rs");
        assert_eq!(found.line, 1);
        assert_eq!(found.column, 2);

        assert!(lookup_location("locations_test_unknown").is_none());
    }
}
