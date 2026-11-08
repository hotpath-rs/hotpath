//! Debug logging - like std::dbg! but tracked in profiler.

use std::fmt::Debug;

use crate::instant::Instant;

use crate::channels::{extract_filename, START_TIME};
use crate::debug::{
    get_sorted_debug_dbg_entries, get_sorted_debug_gauge_entries, get_sorted_debug_val_entries,
    init_debug_state, send_debug_event, DbgEntry, DebugEvent,
};
use crate::json::{format_time_ago, JsonDebugDbgLogs, JsonDebugEntry, JsonDebugList, JsonDebugLog};
use crate::output::format_duration;

fn get_thread_id() -> Option<u64> {
    Some(crate::tid::current_tid())
}

#[doc(hidden)]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
#[inline]
pub fn log_dbg<T: Debug>(id: u32, source: &'static str, expression: &'static str, value: &T) {
    init_debug_state();

    let value_str = crate::output_on::format_debug_truncated(value);
    let timestamp = Instant::now();
    let tid = get_thread_id();

    send_debug_event(DebugEvent::Dbg {
        id,
        source,
        expression,
        value: value_str,
        timestamp,
        tid,
    });
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_debug_entries_json() -> JsonDebugList {
    let dbg_stats = get_sorted_debug_dbg_entries();
    let val_stats = get_sorted_debug_val_entries();
    let gauge_stats = get_sorted_debug_gauge_entries();

    let mut formatted: Vec<JsonDebugEntry> = dbg_stats.iter().map(JsonDebugEntry::from).collect();
    formatted.extend(val_stats.iter().map(JsonDebugEntry::from));
    formatted.extend(gauge_stats.iter().map(JsonDebugEntry::from));

    formatted.sort_by(|a, b| {
        a.entry_type
            .as_str()
            .cmp(b.entry_type.as_str())
            .then(a.source.cmp(&b.source))
            .then(a.expression.cmp(&b.expression))
    });

    let current_elapsed_ns = START_TIME
        .get()
        .map(|t| t.elapsed().as_nanos() as u64)
        .unwrap_or(0);

    JsonDebugList {
        current_elapsed_ns,
        total_count: formatted.len(),
        included_count: formatted.len(),
        entries: formatted,
    }
}

#[cfg(feature = "hotpath-mcp")]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_debug_dbg_entries_json() -> Vec<JsonDebugEntry> {
    get_sorted_debug_dbg_entries()
        .iter()
        .map(JsonDebugEntry::from)
        .collect()
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_dbg_logs(id: u32) -> Option<JsonDebugDbgLogs> {
    let current_elapsed_ns = START_TIME
        .get()
        .map(|t| t.elapsed().as_nanos() as u64)
        .unwrap_or(0);

    crate::debug::get_debug_dbg_entries_by_id(id)
        .map(|s| JsonDebugDbgLogs::from_stats(&s, current_elapsed_ns))
}

fn truncate_source_path(source: &str) -> String {
    if let Some(colon_pos) = source.find(':') {
        let path_part = &source[..colon_pos];
        let suffix = &source[colon_pos..];
        format!("{}{}", extract_filename(path_part), suffix)
    } else {
        extract_filename(source)
    }
}

impl From<&DbgEntry> for JsonDebugEntry {
    fn from(stats: &DbgEntry) -> Self {
        let last_value = stats.logs.back().map(|e| e.value.clone());
        let source = crate::channels::display_source(stats.source);
        JsonDebugEntry {
            id: Some(stats.id),
            entry_type: crate::json::DebugEntryType::Dbg,
            source: source.to_string(),
            source_display: truncate_source_path(source),
            expression: stats.expression.to_string(),
            log_count: stats.log_count,
            last_value,
            location: crate::lib_on::locations::lookup_location(stats.source),
        }
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl JsonDebugDbgLogs {
    pub(crate) fn from_stats(stats: &DbgEntry, current_elapsed_ns: u64) -> Self {
        JsonDebugDbgLogs {
            source: truncate_source_path(crate::channels::display_source(stats.source)),
            expression: stats.expression.to_string(),
            total_logs: stats.log_count,
            logs: stats
                .logs
                .iter()
                .map(|e| JsonDebugLog {
                    index: e.index,
                    timestamp: format_duration(e.timestamp_ns),
                    ago: format_time_ago(current_elapsed_ns.saturating_sub(e.timestamp_ns)),
                    value: e.value.clone(),
                    thread_id: e.tid,
                    source: None,
                })
                .collect(),
        }
    }
}
