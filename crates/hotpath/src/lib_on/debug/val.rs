//! Value metrics - key-value logging for arbitrary values.

use std::fmt::Debug;
use std::sync::Arc;

use crate::instant::Instant;

use crate::channels::{extract_filename, START_TIME};
#[cfg(feature = "hotpath-mcp")]
use crate::debug::get_sorted_debug_val_entries;
use crate::debug::{
    get_or_create_val_id, init_debug_state, send_debug_event, DebugEvent, ValEntry,
};
use crate::json::{format_time_ago, JsonDebugEntry, JsonDebugLog, JsonDebugValLogs};
use crate::output::format_duration;

fn get_thread_id() -> Option<u64> {
    Some(crate::tid::current_tid())
}

pub struct ValHandle {
    id: u32,
    key: Arc<str>,
    source: &'static str,
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl ValHandle {
    #[inline]
    pub fn new(key: impl Into<Arc<str>>, source: &'static str) -> Self {
        init_debug_state();
        let key = key.into();
        let id = get_or_create_val_id(&key);
        Self { id, key, source }
    }

    #[inline]
    pub fn set<T: Debug>(&self, value: &T) {
        let value_str = crate::output_on::format_debug_truncated(value);
        let timestamp = Instant::now();
        let tid = get_thread_id();

        send_debug_event(DebugEvent::Val {
            id: self.id,
            key: self.key.clone(),
            source: self.source,
            value: value_str,
            timestamp,
            tid,
        });
    }
}

#[cfg(feature = "hotpath-mcp")]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_debug_val_entries_json() -> Vec<JsonDebugEntry> {
    get_sorted_debug_val_entries()
        .iter()
        .map(JsonDebugEntry::from)
        .collect()
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_val_logs(id: u32) -> Option<JsonDebugValLogs> {
    let current_elapsed_ns = START_TIME
        .get()
        .map(|t| t.elapsed().as_nanos() as u64)
        .unwrap_or(0);

    crate::debug::get_debug_val_entries_by_id(id)
        .map(|s| JsonDebugValLogs::from_stats(&s, current_elapsed_ns))
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

impl From<&ValEntry> for JsonDebugEntry {
    fn from(stats: &ValEntry) -> Self {
        let last_value = stats.logs.back().map(|e| e.value.clone());
        let location = stats
            .logs
            .back()
            .and_then(|e| crate::lib_on::locations::lookup_location(e.source));
        let (source, source_display) = stats
            .logs
            .back()
            .map(|e| {
                let source = crate::channels::display_source(e.source);
                (source.to_string(), truncate_source_path(source))
            })
            .unwrap_or_else(|| ("<unknown>".to_string(), "<unknown>".to_string()));
        JsonDebugEntry {
            id: Some(stats.id),
            entry_type: crate::json::DebugEntryType::Val,
            source,
            source_display,
            expression: stats.key.to_string(),
            log_count: stats.log_count,
            last_value,
            location,
        }
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl JsonDebugValLogs {
    pub(crate) fn from_stats(stats: &ValEntry, current_elapsed_ns: u64) -> Self {
        JsonDebugValLogs {
            key: stats.key.to_string(),
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
                    source: Some(truncate_source_path(crate::channels::display_source(
                        e.source,
                    ))),
                })
                .collect(),
        }
    }
}
