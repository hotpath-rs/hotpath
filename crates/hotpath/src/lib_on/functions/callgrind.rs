//! Deterministic per-function instruction counts (`hotpath-callgrind`).
//!
//! The feature only changes what the measure macros emit: each measured body
//! runs inside a nested `#[inline(never)] fn __hotpath_cg_<hash>` (sync) or is
//! polled through one (async, via [`CallgrindFuture`]), registered here
//! against the measured name. Whether a run reports instructions is decided
//! at runtime: under `valgrind --tool=callgrind` the session zeroes Callgrind's
//! counters at guard creation, and the guard drop writes a dump through a
//! client request, stops instrumenting and attributes the dump's inclusive
//! `Ir` to measured functions by marker (after the `before_shutdown`
//! callback, so its measured calls count too). Natively the markers are one extra
//! call per measured invocation and nothing else happens.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, OnceLock};
use std::task::{Context, Poll};

use pin_project_lite::pin_project;

use crate::json::{JsonFunctionEntry, JsonFunctionsList};
use crate::lib_on::functions::FunctionStatsConfig;
use crate::output::ProfilingMode;

pub(crate) mod parser;
pub(crate) mod request;

/// Marker ident -> measured names registered for it. More than one name
/// means `macro_rules!` expanded several measured fns from one source
/// position; their counts cannot be told apart.
static MARKERS: OnceLock<crate::lib_on::MetaRwLock<HashMap<&'static str, Vec<&'static str>>>> =
    OnceLock::new();

/// Maps a marker ident emitted by `#[measure]` to the measured name.
#[doc(hidden)]
pub fn register_callgrind_marker(marker: &'static str, name: &'static str) {
    let _suspend = crate::lib_on::SuspendAllocTracking::new();
    let map =
        MARKERS.get_or_init(|| crate::lib_on::meta_rw_lock!("callgrind_markers", HashMap::new()));
    if let Ok(mut w) = map.write() {
        let names = w.entry(marker).or_default();
        if !names.contains(&name) {
            names.push(name);
        }
    }
}

fn markers() -> HashMap<&'static str, Vec<&'static str>> {
    MARKERS
        .get()
        .and_then(|m| m.read().ok().map(|g| g.clone()))
        .unwrap_or_default()
}

pin_project! {
    /// Polls a measured async body through the call-site marker function, so
    /// the body's poll has a symbol of its own instead of being inlined into
    /// the executor.
    #[doc(hidden)]
    pub struct CallgrindFuture<F, P> {
        #[pin]
        fut: F,
        poll_fn: P,
    }
}

impl<F, P> CallgrindFuture<F, P>
where
    F: Future,
    P: FnMut(Pin<&mut F>, &mut Context<'_>) -> Poll<F::Output>,
{
    #[doc(hidden)]
    pub fn new(fut: F, poll_fn: P) -> Self {
        Self { fut, poll_fn }
    }
}

impl<F, P> Future for CallgrindFuture<F, P>
where
    F: Future,
    P: FnMut(Pin<&mut F>, &mut Context<'_>) -> Poll<F::Output>,
{
    type Output = F::Output;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let this = self.project();
        (this.poll_fn)(this.fut, cx)
    }
}

/// Whether this run counts instructions (the process runs under Valgrind).
pub(crate) fn active() -> bool {
    *request::ON_VALGRIND
}

/// `HOTPATH_CALLGRIND_OUT_FILE`: the `--callgrind-out-file` value Valgrind
/// was started with (`%p` and `%q{VAR}` expanded like Valgrind does), default
/// `callgrind.out.%p`. Relative paths resolve against the working directory
/// at the first guard creation, like Valgrind's against its startup cwd.
static OUT_FILE: LazyLock<PathBuf> = LazyLock::new(|| {
    let template = std::env::var("HOTPATH_CALLGRIND_OUT_FILE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "callgrind.out.%p".to_string());
    let path = PathBuf::from(expand_out_file(&template, std::process::id()));
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&path))
            .unwrap_or(path)
    }
});

fn expand_out_file(template: &str, pid: u32) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(pos) = rest.find('%') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        if let Some(after) = rest.strip_prefix("%p") {
            out.push_str(&pid.to_string());
            rest = after;
        } else if let Some((var, after)) = rest.strip_prefix("%q{").and_then(|r| r.split_once('}'))
        {
            out.push_str(&std::env::var(var).unwrap_or_default());
            rest = after;
        } else if let Some(after) = rest.strip_prefix("%%") {
            out.push('%');
            rest = after;
        } else {
            out.push('%');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Client-request dumps this process wrote; Callgrind numbers them `.1`,
/// `.2`, ... in the same order.
static DUMPS: AtomicU32 = AtomicU32::new(0);

/// Called at guard creation.
pub(crate) fn start_session() {
    if active() {
        LazyLock::force(&OUT_FILE);
        request::start_session();
    }
}

pub(crate) struct IcountReport {
    pub(crate) total_ir: u64,
    /// Inclusive `Ir` per measured name.
    pub(crate) by_name: HashMap<&'static str, u64>,
    /// `creator:` of the dump, e.g. `callgrind-3.24.0`.
    pub(crate) tool: Option<String>,
}

/// Called first thing at guard drop. `None` when not under Valgrind.
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
pub(crate) fn finish_session() -> Option<Result<IcountReport, String>> {
    if !active() {
        return None;
    }
    let dump_no = DUMPS.fetch_add(1, Ordering::Relaxed) + 1;
    let tag = format!("hotpath-{}-{dump_no}", std::process::id());
    let tag_c = std::ffi::CString::new(tag.as_str()).expect("tag has no NUL");
    request::dump_and_stop(&tag_c);

    let mut path = OUT_FILE.clone().into_os_string();
    path.push(format!(".{dump_no}"));
    let path = PathBuf::from(path);
    Some(read_report(&path, &tag))
}

fn read_report(path: &std::path::Path, tag: &str) -> Result<IcountReport, String> {
    let input = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "failed to read Callgrind dump {}: {e} (is valgrind running with \
             --tool=callgrind? With a custom --callgrind-out-file, set \
             HOTPATH_CALLGRIND_OUT_FILE to the same value)",
            path.display()
        )
    })?;
    let dump = parser::parse(&input)
        .map_err(|e| format!("failed to parse Callgrind dump {}: {e}", path.display()))?;

    let expected = format!("Client Request: {tag}");
    if dump.trigger.as_deref() != Some(expected.as_str()) {
        return Err(format!(
            "Callgrind dump {} was not written by hotpath (trigger {:?}); other dumps \
             (--dump-every-bb, --dump-before, client requests) shift Callgrind's numbering",
            path.display(),
            dump.trigger.as_deref().unwrap_or("none"),
        ));
    }

    let markers = markers();
    let mut by_name: HashMap<&'static str, u64> = HashMap::new();
    for (marker, ir) in &dump.inclusive {
        match markers.get(marker.as_str()).map(Vec::as_slice) {
            Some([name]) => *by_name.entry(name).or_insert(0) += ir,
            Some(names) if !names.is_empty() => {
                eprintln!(
                    "[hotpath] no instruction counts for {}: generated from one source \
                     position, so their counts cannot be told apart",
                    names.join(", ")
                );
            }
            _ => {}
        }
    }

    Ok(IcountReport {
        total_ir: dump.total_ir,
        by_name,
        tool: dump.creator,
    })
}

/// Calls and id per measured name, from the timing or alloc stats.
pub(crate) type CallCounts = HashMap<&'static str, (u32, u64)>;

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure)]
pub(crate) fn build_icount_list(
    report: &IcountReport,
    calls: &CallCounts,
    config: &FunctionStatsConfig,
    current_elapsed_ns: u64,
) -> JsonFunctionsList {
    let precision = crate::output::Precision::for_cloud(config.cloud);

    let mut entries: Vec<(&'static str, u64, u32, u64)> = report
        .by_name
        .iter()
        .filter_map(|(&name, &ir)| {
            // Unfocused or never-called functions have no stats entry.
            let &(id, count) = calls.get(name)?;
            Some((name, ir, id, count))
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    let total_count = entries.len();
    if config.limit > 0 {
        entries.truncate(config.limit);
    }
    let included_count = entries.len();

    let data = entries
        .into_iter()
        .map(|(name, ir, id, count)| {
            let percentage = if report.total_ir > 0 {
                (ir as f64 / report.total_ir as f64) * 100.0
            } else {
                0.0
            };
            JsonFunctionEntry {
                id,
                name: name.to_string(),
                calls: count,
                sampled_calls: count,
                avg: ir
                    .checked_div(count)
                    .map_or_else(|| "-".to_string(), |avg| avg.to_string()),
                percentiles: HashMap::new(),
                total: ir.to_string(),
                percent_total: format!("{percentage:.2}%"),
                location: crate::lib_on::locations::lookup_location(name),
            }
        })
        .collect();

    JsonFunctionsList {
        profiling_mode: ProfilingMode::Icount,
        time_elapsed: precision.duration(config.total_elapsed.as_nanos() as u64),
        total_elapsed_ns: current_elapsed_ns,
        total_allocated: None,
        description: "Inclusive instructions executed per function (Callgrind Ir).".to_string(),
        caller_name: config.caller_name.to_string(),
        percentiles: Vec::new(),
        data,
        total_count,
        included_count,
        tool: report.tool.clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::lib_on::functions::callgrind::expand_out_file;

    #[test]
    fn out_file_expands_like_valgrind() {
        assert_eq!(expand_out_file("callgrind.out.%p", 42), "callgrind.out.42");
        assert_eq!(
            expand_out_file("/tmp/%q{HOTPATH_CG_UNSET_VAR}cg.%p.%%x%z%", 7),
            "/tmp/cg.7.%x%z%"
        );
    }
}
