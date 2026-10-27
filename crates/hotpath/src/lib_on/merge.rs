//! `HOTPATH_MERGE_REPORT`: a Callgrind run merges the JSON report of a native
//! run of the same build into its own, so one report carries native timing
//! (and every other native section) next to the instruction counts.
//!
//! The native report is the base; the Callgrind run contributes only
//! `functions_icount` and sets `meta.run` to `merged`. Both runs must be the
//! same build, checkout, benchmark and CI run, checked field by field, so a
//! stale or foreign file is refused rather than merged. A requested merge
//! that fails writes no JSON, uploads nothing and fails the process,
//! whatever `HOTPATH_UPLOAD` says.
//!
//! Checks run input file first (read, parse, kind, timing, identity), then
//! this process (under Callgrind, dump read, icount section), so every
//! failure about the file is reachable without Valgrind.

use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::json::cloud_api::MAX_REPORT_BYTES;
use crate::json::{JsonReport, JsonRun};
use crate::shared::{CRITERION_BENCHMARK_KEY, CRITERION_ITERATIONS_KEY};

/// `HOTPATH_MERGE_REPORT`: path of the native run's JSON report. Blank or
/// unset requests no merge; relative paths resolve against the cwd.
pub(crate) static MERGE_REPORT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    std::env::var_os("HOTPATH_MERGE_REPORT")
        .filter(|p| !p.is_empty())
        .map(crate::output_on::resolve_output_path)
});

/// What this (the Callgrind) process measured.
#[cfg_attr(not(feature = "hotpath-callgrind"), allow(dead_code))]
pub(crate) enum IcountRun<'a> {
    /// Not running under Valgrind.
    NotUnderValgrind,
    /// Under Valgrind, but the Callgrind dump could not be read.
    DumpFailed(&'a str),
    /// Under Valgrind with the dump read; the section is in the report.
    Counted,
}

/// Longest field value quoted in a mismatch message.
const MAX_QUOTED: usize = 48;

#[derive(Debug)]
pub(crate) enum MergeError {
    MissingFeatures,
    NothingToMergeInto,
    SamePath(PathBuf),
    Read {
        path: PathBuf,
        error: String,
    },
    InputTooLarge {
        path: PathBuf,
    },
    Parse {
        path: PathBuf,
        error: String,
    },
    Version {
        native: String,
        own: String,
    },
    NativeKind(JsonRun),
    NativeHasIcount,
    NativeTiming(&'static str),
    Mismatch {
        field: &'static str,
        native: Option<String>,
        own: Option<String>,
    },
    NotUnderValgrind,
    DumpFailed(String),
    NoIcount(&'static str),
    Tool(Option<String>),
    TooLarge(String),
    Serialize(String),
}

fn quote(value: &Option<String>) -> String {
    match value {
        None => "none".to_string(),
        Some(v) if v.chars().count() > MAX_QUOTED => {
            let cut: String = v.chars().take(MAX_QUOTED).collect();
            format!("{cut:?}...")
        }
        Some(v) => format!("{v:?}"),
    }
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::MissingFeatures => write!(
                f,
                "needs the hotpath-callgrind and hotpath-cloud features in this build"
            ),
            MergeError::NothingToMergeInto => write!(
                f,
                "nothing to merge into: this run writes no JSON report and uploads none \
                 (set HOTPATH_OUTPUT_FORMAT=json or HOTPATH_UPLOAD)"
            ),
            MergeError::SamePath(path) => write!(
                f,
                "HOTPATH_OUTPUT_PATH and HOTPATH_MERGE_REPORT both name {}: write the merged \
                 report to another file",
                path.display()
            ),
            MergeError::Read { path, error } => write!(
                f,
                "cannot read the native report {}: {error} (run the benchmark natively with \
                 HOTPATH_OUTPUT_FORMAT=json and HOTPATH_OUTPUT_PATH set to this path first)",
                path.display()
            ),
            MergeError::InputTooLarge { path } => write!(
                f,
                "the native report {} is over the {MAX_REPORT_BYTES} byte report limit",
                path.display()
            ),
            MergeError::Parse { path, error } => write!(
                f,
                "the native report {} is not a hotpath JSON report: {error}",
                path.display()
            ),
            MergeError::Version { native, own } => write!(
                f,
                "the native report was written by hotpath {native}, this run is hotpath {own}: \
                 run both from the same build"
            ),
            MergeError::NativeKind(run) => write!(
                f,
                "the native report comes from a {} run, not a native one",
                match run {
                    JsonRun::Native => "native",
                    JsonRun::Callgrind => "Callgrind",
                    JsonRun::Merged => "merged",
                }
            ),
            MergeError::NativeHasIcount => write!(
                f,
                "the native report already carries instruction counts: point \
                 HOTPATH_MERGE_REPORT at the report of a native run"
            ),
            MergeError::NativeTiming(why) => write!(
                f,
                "the native report has no timing to merge ({why}): run it with function \
                 timing on (a nonzero sampling rate, timing in HOTPATH_REPORT)"
            ),
            MergeError::Mismatch { field, native, own } => write!(
                f,
                "the native report's {field} is {}, this run's is {}: run both from the same \
                 build, checkout and CI job",
                quote(native),
                quote(own)
            ),
            MergeError::NotUnderValgrind => write!(
                f,
                "this run is not under valgrind --tool=callgrind: set HOTPATH_MERGE_REPORT only \
                 on the Callgrind run"
            ),
            MergeError::DumpFailed(msg) => write!(f, "no instruction counts: {msg}"),
            MergeError::NoIcount(why) => write!(
                f,
                "no instruction counts to merge ({why}; keep functions-icount in HOTPATH_REPORT)"
            ),
            MergeError::Tool(tool) => write!(
                f,
                "the instruction counts name no Callgrind version (tool {})",
                quote(tool)
            ),
            MergeError::TooLarge(msg) => write!(f, "the merged report: {msg}"),
            MergeError::Serialize(msg) => {
                write!(f, "the merged report cannot be serialized: {msg}")
            }
        }
    }
}

/// The merged report, or why it cannot be built. `output` is the JSON
/// destination file, if any, refused when it is the native report itself.
pub(crate) fn apply(
    own: JsonReport,
    icount: IcountRun<'_>,
    native_path: &Path,
    output: Option<&Path>,
) -> Result<JsonReport, MergeError> {
    if !cfg!(all(
        feature = "hotpath-callgrind",
        feature = "hotpath-cloud"
    )) {
        return Err(MergeError::MissingFeatures);
    }
    if output.is_some_and(|out| same_file(out, native_path)) {
        return Err(MergeError::SamePath(native_path.to_path_buf()));
    }
    merge(read_native(native_path)?, own, icount)
}

/// Equal paths, or the same file once both exist.
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Reads and parses the native report, refusing one over the report limit
/// before parsing it.
pub(crate) fn read_native(path: &Path) -> Result<JsonReport, MergeError> {
    let read_error = |e: std::io::Error| MergeError::Read {
        path: path.to_path_buf(),
        error: e.to_string(),
    };
    let file = std::fs::File::open(path).map_err(read_error)?;
    let mut bytes = Vec::new();
    file.take(MAX_REPORT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() > MAX_REPORT_BYTES {
        return Err(MergeError::InputTooLarge {
            path: path.to_path_buf(),
        });
    }
    serde_json::from_slice(&bytes).map_err(|e| MergeError::Parse {
        path: path.to_path_buf(),
        error: e.to_string(),
    })
}

/// `native` with `own`'s instruction counts, after every check.
pub(crate) fn merge(
    mut native: JsonReport,
    own: JsonReport,
    icount: IcountRun<'_>,
) -> Result<JsonReport, MergeError> {
    if native.version != own.version {
        return Err(MergeError::Version {
            native: native.version,
            own: own.version,
        });
    }
    if native.functions_icount.is_some() {
        return Err(match native.run() {
            JsonRun::Native => MergeError::NativeHasIcount,
            run => MergeError::NativeKind(run),
        });
    }
    match native.run() {
        JsonRun::Native => {}
        run => return Err(MergeError::NativeKind(run)),
    }
    let timing = native
        .functions_timing
        .as_ref()
        .ok_or(MergeError::NativeTiming("no functions_timing section"))?;
    if timing.data.is_empty() {
        return Err(MergeError::NativeTiming("functions_timing is empty"));
    }
    if timing.total_elapsed_ns == 0 {
        return Err(MergeError::NativeTiming(
            "functions_timing has no elapsed time",
        ));
    }
    check_identity(&native, &own)?;

    match icount {
        IcountRun::NotUnderValgrind => return Err(MergeError::NotUnderValgrind),
        IcountRun::DumpFailed(msg) => return Err(MergeError::DumpFailed(msg.to_string())),
        IcountRun::Counted => {}
    }
    let list = own
        .functions_icount
        .ok_or(MergeError::NoIcount("no functions_icount section"))?;
    if list.data.is_empty() || list.total_count == 0 {
        return Err(MergeError::NoIcount("functions_icount is empty"));
    }
    let versioned = list
        .tool
        .as_deref()
        .and_then(|t| t.strip_prefix("callgrind-"))
        .is_some_and(|v| !v.is_empty());
    if !versioned {
        return Err(MergeError::Tool(list.tool));
    }

    native.functions_icount = Some(list);
    native.meta.run = Some(JsonRun::Merged);
    Ok(native)
}

/// Every field that places a run: build, checkout, benchmark, policy, CI run
/// and Criterion workload. Absent on both sides is equal (a local run has no
/// CI fields); absent on one side is a mismatch.
fn check_identity(native: &JsonReport, own: &JsonReport) -> Result<(), MergeError> {
    let (n, o) = (&native.meta, &own.meta);
    let s = |v: &str| Some(v.to_string());
    let git = |m: &crate::json::JsonMeta| m.git.clone();
    let ci = |m: &crate::json::JsonMeta| m.ci.clone();
    let pr = |m: &crate::json::JsonMeta| m.ci.as_ref().and_then(|c| c.pull_request.clone());
    let (ng, og, nc, oc, np, op) = (git(n), git(o), ci(n), ci(o), pr(n), pr(o));
    let user = |r: &JsonReport, key: &str| r.user_metadata.as_ref()?.get(key).cloned();

    let fields: [(&'static str, Option<String>, Option<String>); 22] = [
        ("benchmark", n.benchmark.clone(), o.benchmark.clone()),
        ("rustc", s(&n.rustc), s(&o.rustc)),
        ("os", s(&n.os), s(&o.os)),
        (
            "policy path",
            n.policy.as_ref().map(|p| p.path.clone()),
            o.policy.as_ref().map(|p| p.path.clone()),
        ),
        ("source_root", n.source_root.clone(), o.source_root.clone()),
        (
            "git sha",
            ng.as_ref().map(|g| g.sha.clone()),
            og.as_ref().map(|g| g.sha.clone()),
        ),
        (
            "git ref",
            ng.as_ref().and_then(|g| g.r#ref.clone()),
            og.as_ref().and_then(|g| g.r#ref.clone()),
        ),
        (
            "git base_sha",
            ng.as_ref().and_then(|g| g.base_sha.clone()),
            og.as_ref().and_then(|g| g.base_sha.clone()),
        ),
        (
            "git repository",
            ng.as_ref().and_then(|g| g.repository.clone()),
            og.as_ref().and_then(|g| g.repository.clone()),
        ),
        (
            "ci provider",
            nc.as_ref().map(|c| c.provider.clone()),
            oc.as_ref().map(|c| c.provider.clone()),
        ),
        (
            "ci event",
            nc.as_ref().map(|c| c.event.clone()),
            oc.as_ref().map(|c| c.event.clone()),
        ),
        (
            "ci run_id",
            nc.as_ref().and_then(|c| c.run_id.clone()),
            oc.as_ref().and_then(|c| c.run_id.clone()),
        ),
        (
            "ci run_attempt",
            nc.as_ref().and_then(|c| c.run_attempt.clone()),
            oc.as_ref().and_then(|c| c.run_attempt.clone()),
        ),
        (
            "ci workflow",
            nc.as_ref().and_then(|c| c.workflow.clone()),
            oc.as_ref().and_then(|c| c.workflow.clone()),
        ),
        (
            "ci repository_id",
            nc.as_ref().and_then(|c| c.repository_id.clone()),
            oc.as_ref().and_then(|c| c.repository_id.clone()),
        ),
        (
            "pull request number",
            np.as_ref().map(|p| p.number.to_string()),
            op.as_ref().map(|p| p.number.to_string()),
        ),
        (
            "pull request base_ref",
            np.as_ref().map(|p| p.base_ref.clone()),
            op.as_ref().map(|p| p.base_ref.clone()),
        ),
        (
            "pull request head_ref",
            np.as_ref().map(|p| p.head_ref.clone()),
            op.as_ref().map(|p| p.head_ref.clone()),
        ),
        (
            "pull request head_sha",
            np.as_ref().and_then(|p| p.head_sha.clone()),
            op.as_ref().and_then(|p| p.head_sha.clone()),
        ),
        (
            "policy",
            n.policy.as_ref().map(|p| p.source.clone()),
            o.policy.as_ref().map(|p| p.source.clone()),
        ),
        (
            "Criterion benchmark",
            user(native, CRITERION_BENCHMARK_KEY),
            user(own, CRITERION_BENCHMARK_KEY),
        ),
        (
            "Criterion iteration count",
            user(native, CRITERION_ITERATIONS_KEY),
            user(own, CRITERION_ITERATIONS_KEY),
        ),
    ];
    for (field, native, own) in fields {
        if native != own {
            return Err(MergeError::Mismatch { field, native, own });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::json::{
        JsonCiInfo, JsonFunctionEntry, JsonFunctionsList, JsonGitInfo, JsonMeta, JsonPolicy,
        JsonPullRequest, JsonReport, JsonRun,
    };
    use crate::lib_on::merge::{merge, read_native, IcountRun, MergeError};
    use crate::output::ProfilingMode;

    fn entry(name: &str, calls: u64) -> JsonFunctionEntry {
        serde_json::from_value(serde_json::json!({
            "id": 1, "name": name, "calls": calls, "sampled_calls": calls, "avg": "1", "total": "1",
            "percent_total": "100.00%"
        }))
        .unwrap()
    }

    fn list(mode: ProfilingMode, tool: Option<&str>, calls: u64) -> JsonFunctionsList {
        JsonFunctionsList {
            profiling_mode: mode,
            time_elapsed: "1.00 s".to_string(),
            total_elapsed_ns: 1_000_000_000,
            total_allocated: None,
            description: String::new(),
            caller_name: "main".to_string(),
            percentiles: Vec::new(),
            data: vec![entry("app::work", calls)],
            total_count: 1,
            included_count: 1,
            tool: tool.map(str::to_string),
        }
    }

    fn meta(run: JsonRun) -> JsonMeta {
        JsonMeta {
            rustc: "1.99.0".to_string(),
            os: "linux-x86_64".to_string(),
            created_at: time::macros::datetime!(2026-10-24 10:00:00 UTC),
            source_root: Some(String::new()),
            git: Some(JsonGitInfo {
                sha: "3f1c".to_string(),
                r#ref: Some("refs/pull/7/merge".to_string()),
                base_sha: Some("b0b0".to_string()),
                repository: Some("o/r".to_string()),
            }),
            ci: Some(JsonCiInfo {
                provider: "github-actions".to_string(),
                event: "pull_request".to_string(),
                pull_request: Some(JsonPullRequest {
                    number: 7,
                    base_ref: "main".to_string(),
                    head_ref: "feature".to_string(),
                    head_sha: Some("a1a1".to_string()),
                }),
                run_id: Some("100".to_string()),
                run_attempt: Some("1".to_string()),
                workflow: Some("Benchmark".to_string()),
                actor: Some("someone".to_string()),
                repository_id: Some("42".to_string()),
            }),
            benchmark: Some("bench".to_string()),
            policy: Some(JsonPolicy {
                source: "[functions.icount]\n".to_string(),
                path: "hotpath/policy.toml".to_string(),
            }),
            run: Some(run),
        }
    }

    fn native() -> JsonReport {
        JsonReport {
            functions_timing: Some(list(ProfilingMode::Timing, None, 3)),
            functions_alloc: Some(list(ProfilingMode::AllocBytes, None, 3)),
            ..JsonReport::new(meta(JsonRun::Native))
        }
    }

    fn own() -> JsonReport {
        JsonReport {
            functions_icount: Some(list(ProfilingMode::Icount, Some("callgrind-3.26.0"), 5)),
            functions_alloc: Some(list(ProfilingMode::AllocBytes, None, 99)),
            ..JsonReport::new(meta(JsonRun::Callgrind))
        }
    }

    fn err(native: JsonReport, own: JsonReport) -> MergeError {
        merge(native, own, IcountRun::Counted).unwrap_err()
    }

    #[test]
    fn merged_report_is_native_plus_icount() {
        let merged = merge(native(), own(), IcountRun::Counted).unwrap();
        assert_eq!(merged.meta.run, Some(JsonRun::Merged));
        assert_eq!(merged.run(), JsonRun::Merged);
        let icount = merged.functions_icount.expect("icount");
        assert_eq!(icount.data[0].calls, 5, "icount keeps its own calls");
        assert_eq!(merged.functions_timing.expect("timing").data[0].calls, 3);
        assert_eq!(
            merged.functions_alloc.expect("alloc").data[0].calls,
            3,
            "allocations come from the native run"
        );
    }

    #[test]
    fn local_runs_without_ci_fields_merge() {
        let local = |mut r: JsonReport| {
            r.meta.ci = None;
            r.meta.git = None;
            r
        };
        assert!(merge(local(native()), local(own()), IcountRun::Counted).is_ok());
    }

    #[test]
    fn every_identity_field_is_compared() {
        type Edit = fn(&mut JsonMeta);
        let edits: [(&str, Edit); 20] = [
            ("benchmark", |m| m.benchmark = Some("other".into())),
            ("rustc", |m| m.rustc = "1.98.0".into()),
            ("os", |m| m.os = "macos-aarch64".into()),
            ("policy path", |m| {
                m.policy.as_mut().unwrap().path = "hotpath/x-policy.toml".into()
            }),
            ("source_root", |m| m.source_root = Some("crates/app".into())),
            ("git sha", |m| m.git.as_mut().unwrap().sha = "9a0b".into()),
            ("git ref", |m| m.git.as_mut().unwrap().r#ref = None),
            ("git base_sha", |m| m.git.as_mut().unwrap().base_sha = None),
            ("git repository", |m| {
                m.git.as_mut().unwrap().repository = Some("o/other".into())
            }),
            ("ci provider", |m| m.ci = None),
            ("ci event", |m| m.ci.as_mut().unwrap().event = "push".into()),
            ("ci run_id", |m| {
                m.ci.as_mut().unwrap().run_id = Some("101".into())
            }),
            ("ci run_attempt", |m| {
                m.ci.as_mut().unwrap().run_attempt = Some("2".into())
            }),
            ("ci workflow", |m| m.ci.as_mut().unwrap().workflow = None),
            ("ci repository_id", |m| {
                m.ci.as_mut().unwrap().repository_id = Some("43".into())
            }),
            ("pull request number", |m| {
                m.ci.as_mut().unwrap().pull_request.as_mut().unwrap().number = 8
            }),
            ("pull request base_ref", |m| {
                m.ci.as_mut()
                    .unwrap()
                    .pull_request
                    .as_mut()
                    .unwrap()
                    .base_ref = "dev".into()
            }),
            ("pull request head_ref", |m| {
                m.ci.as_mut()
                    .unwrap()
                    .pull_request
                    .as_mut()
                    .unwrap()
                    .head_ref = "x".into()
            }),
            ("pull request head_sha", |m| {
                m.ci.as_mut()
                    .unwrap()
                    .pull_request
                    .as_mut()
                    .unwrap()
                    .head_sha = None
            }),
            ("policy", |m| {
                m.policy.as_mut().unwrap().source = "[functions.timing]\n".into()
            }),
        ];
        for (expected, edit) in edits {
            let mut native = native();
            edit(&mut native.meta);
            match err(native, own()) {
                MergeError::Mismatch { field, .. } => assert_eq!(field, expected),
                other => panic!("{expected}: {other}"),
            }
        }
        // The actor is the same person for one run; it is not compared.
        let mut native = native();
        native.meta.ci.as_mut().unwrap().actor = Some("other".into());
        assert!(merge(native, own(), IcountRun::Counted).is_ok());
    }

    #[test]
    fn criterion_workload_must_match() {
        fn with(mut r: JsonReport, pairs: &[(&str, &str)]) -> JsonReport {
            let mut metadata = std::collections::BTreeMap::from([("runner".into(), "ci".into())]);
            metadata.extend(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())));
            r.user_metadata = Some(metadata);
            r
        }
        const ID: &str = "criterion_benchmark";
        const N: &str = "criterion_iterations";
        let a = [(ID, "codec/decode/1024"), (N, "1000")];

        let merged = merge(with(native(), &a), with(own(), &a), IcountRun::Counted);
        assert!(merged.is_ok(), "same benchmark and count merge");

        let cases = [
            (
                [(ID, "codec/decode/1024"), (N, "500")],
                "Criterion iteration count",
            ),
            (
                [(ID, "codec/decode/16"), (N, "1000")],
                "Criterion benchmark",
            ),
        ];
        for (other, expected) in cases {
            match err(with(native(), &a), with(own(), &other)) {
                MergeError::Mismatch { field, .. } => assert_eq!(field, expected),
                other => panic!("{expected}: {other}"),
            }
        }

        // Criterion on one side only, either way round.
        for (n, o) in [(&a[..], &[][..]), (&[][..], &a[..])] {
            match err(with(native(), n), with(own(), o)) {
                MergeError::Mismatch { field, native, own } => {
                    assert_eq!(field, "Criterion benchmark");
                    assert_ne!(native.is_some(), own.is_some());
                }
                other => panic!("{other}"),
            }
        }

        // Neither is a Criterion run: unchanged, other metadata is not compared.
        let mut plain = with(own(), &[]);
        let metadata = plain.user_metadata.as_mut().unwrap();
        metadata.insert("runner".into(), "local".into());
        assert!(merge(with(native(), &[]), plain, IcountRun::Counted).is_ok());
        assert!(merge(native(), own(), IcountRun::Counted).is_ok());
    }

    #[test]
    fn native_file_must_be_a_native_run_with_timing() {
        let mut v = native();
        v.version = "0.0.1".into();
        assert!(matches!(err(v, own()), MergeError::Version { .. }));

        for run in [JsonRun::Callgrind, JsonRun::Merged] {
            let mut r = native();
            r.meta.run = Some(run);
            assert!(matches!(err(r, own()), MergeError::NativeKind(k) if k == run));
        }
        // Older than `meta.run`: a report with icount resolves to Callgrind.
        let mut legacy = native();
        legacy.meta.run = None;
        legacy.functions_icount = own().functions_icount;
        assert!(matches!(
            err(legacy, own()),
            MergeError::NativeKind(JsonRun::Callgrind)
        ));
        // Explicitly native, but carrying counts.
        let mut counted = native();
        counted.functions_icount = own().functions_icount;
        assert!(matches!(err(counted, own()), MergeError::NativeHasIcount));
        // Older than `meta.run` without icount resolves to native and merges.
        let mut legacy_native = native();
        legacy_native.meta.run = None;
        assert!(merge(legacy_native, own(), IcountRun::Counted).is_ok());

        let mut none = native();
        none.functions_timing = None;
        assert!(matches!(err(none, own()), MergeError::NativeTiming(_)));
        let mut empty = native();
        empty.functions_timing.as_mut().unwrap().data.clear();
        assert!(matches!(err(empty, own()), MergeError::NativeTiming(_)));
        let mut zero = native();
        zero.functions_timing.as_mut().unwrap().total_elapsed_ns = 0;
        assert!(matches!(err(zero, own()), MergeError::NativeTiming(_)));
    }

    #[test]
    fn this_run_must_have_counted_instructions() {
        assert!(matches!(
            merge(native(), own(), IcountRun::NotUnderValgrind).unwrap_err(),
            MergeError::NotUnderValgrind
        ));
        let failed = merge(native(), own(), IcountRun::DumpFailed("no dump")).unwrap_err();
        assert!(failed.to_string().contains("no dump"), "{failed}");

        let mut missing = own();
        missing.functions_icount = None;
        assert!(matches!(err(native(), missing), MergeError::NoIcount(_)));
        let mut empty = own();
        empty.functions_icount.as_mut().unwrap().data.clear();
        assert!(matches!(err(native(), empty), MergeError::NoIcount(_)));
        for tool in [None, Some("cachegrind-3.26.0"), Some("callgrind-")] {
            let mut r = own();
            r.functions_icount.as_mut().unwrap().tool = tool.map(str::to_string);
            assert!(matches!(err(native(), r), MergeError::Tool(_)), "{tool:?}");
        }
    }

    #[test]
    fn native_file_is_read_size_capped() {
        let dir = std::env::temp_dir().join(format!("hotpath-merge-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let missing = read_native(&dir.join("missing.json")).unwrap_err();
        assert!(matches!(missing, MergeError::Read { .. }), "{missing}");

        let malformed = dir.join("malformed.json");
        std::fs::write(&malformed, "{\"type\":").unwrap();
        assert!(matches!(
            read_native(&malformed).unwrap_err(),
            MergeError::Parse { .. }
        ));

        let huge = dir.join("huge.json");
        let limit = crate::json::cloud_api::MAX_REPORT_BYTES;
        std::fs::write(&huge, vec![b' '; limit + 1]).unwrap();
        assert!(matches!(
            read_native(&huge).unwrap_err(),
            MergeError::InputTooLarge { .. }
        ));
        // At the limit it is read and parsed (whitespace is not a report).
        std::fs::write(&huge, vec![b' '; limit]).unwrap();
        assert!(matches!(
            read_native(&huge).unwrap_err(),
            MergeError::Parse { .. }
        ));

        let good = dir.join("native.json");
        std::fs::write(&good, serde_json::to_vec(&native()).unwrap()).unwrap();
        assert!(read_native(&good).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
