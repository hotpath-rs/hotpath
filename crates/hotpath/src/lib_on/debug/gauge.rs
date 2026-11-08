//! Gauge metrics - numeric values with set/inc/dec operations.

use std::collections::VecDeque;
use std::sync::Arc;

pub use crate::shared::IntoF64;

use crate::instant::Instant;

use crate::channels::{extract_filename, LOGS_LIMIT, START_TIME};
use crate::debug::{get_or_create_gauge_id, init_debug_state, send_debug_event, DebugEvent};
use crate::json::{format_time_ago, JsonDebugEntry, JsonDebugGaugeLogs, JsonDebugLog};
use crate::output::format_duration;

fn get_thread_id() -> Option<u64> {
    Some(crate::tid::current_tid())
}

#[derive(Debug, Clone)]
pub(crate) struct GaugeEntry {
    pub id: u32,
    pub key: Arc<str>,
    pub source: &'static str,
    pub current_value: f64,
    pub min_value: f64,
    pub max_value: f64,
    pub update_count: u64,
    pub logs: VecDeque<GaugeLog>,
}

#[derive(Debug, Clone)]
pub(crate) struct GaugeLog {
    pub index: u64,
    pub timestamp_ns: u64,
    pub value: f64,
    pub tid: Option<u64>,
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl GaugeEntry {
    pub fn new(id: u32, key: Arc<str>, source: &'static str, initial_value: f64) -> Self {
        Self {
            id,
            key,
            source,
            current_value: initial_value,
            min_value: initial_value,
            max_value: initial_value,
            update_count: 0,
            logs: VecDeque::with_capacity(*LOGS_LIMIT),
        }
    }
}

pub struct GaugeHandle {
    id: u32,
    key: Arc<str>,
    source: &'static str,
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl GaugeHandle {
    #[inline]
    pub fn new(key: impl Into<Arc<str>>, source: &'static str) -> Self {
        init_debug_state();
        let key = key.into();
        let id = get_or_create_gauge_id(&key);
        Self { id, key, source }
    }

    #[inline]
    pub fn set(&self, value: impl IntoF64) -> &Self {
        let timestamp = Instant::now();
        let tid = get_thread_id();

        send_debug_event(DebugEvent::Gauge {
            id: self.id,
            key: self.key.clone(),
            source: self.source,
            value: value.into_f64(),
            timestamp,
            tid,
        });
        self
    }

    #[inline]
    pub fn inc(&self, delta: impl IntoF64) -> &Self {
        let timestamp = Instant::now();
        let tid = get_thread_id();

        send_debug_event(DebugEvent::GaugeInc {
            id: self.id,
            key: self.key.clone(),
            source: self.source,
            delta: delta.into_f64(),
            timestamp,
            tid,
        });
        self
    }

    #[inline]
    pub fn dec(&self, delta: impl IntoF64) -> &Self {
        let timestamp = Instant::now();
        let tid = get_thread_id();

        send_debug_event(DebugEvent::GaugeDec {
            id: self.id,
            key: self.key.clone(),
            source: self.source,
            delta: delta.into_f64(),
            timestamp,
            tid,
        });
        self
    }
}

#[cfg(feature = "hotpath-mcp")]
#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_debug_gauge_entries_json() -> Vec<JsonDebugEntry> {
    crate::debug::get_sorted_debug_gauge_entries()
        .iter()
        .map(JsonDebugEntry::from)
        .collect()
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_debug_gauge_logs(id: u32) -> Option<JsonDebugGaugeLogs> {
    let current_elapsed_ns = START_TIME
        .get()
        .map(|t| t.elapsed().as_nanos() as u64)
        .unwrap_or(0);

    crate::debug::get_debug_gauge_entries_by_id(id)
        .map(|s| JsonDebugGaugeLogs::from_stats(&s, current_elapsed_ns))
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

impl From<&GaugeEntry> for JsonDebugEntry {
    fn from(stats: &GaugeEntry) -> Self {
        let source = crate::channels::display_source(stats.source);
        JsonDebugEntry {
            id: Some(stats.id),
            entry_type: crate::json::DebugEntryType::Gauge,
            source: source.to_string(),
            source_display: truncate_source_path(source),
            expression: stats.key.to_string(),
            log_count: stats.update_count,
            last_value: Some(format!("{}", stats.current_value)),
            location: crate::lib_on::locations::lookup_location(stats.source),
        }
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure_all)]
impl JsonDebugGaugeLogs {
    pub(crate) fn from_stats(stats: &GaugeEntry, current_elapsed_ns: u64) -> Self {
        JsonDebugGaugeLogs {
            key: stats.key.to_string(),
            total_logs: stats.update_count,
            logs: stats
                .logs
                .iter()
                .map(|e| JsonDebugLog {
                    index: e.index,
                    timestamp: format_duration(e.timestamp_ns),
                    ago: format_time_ago(current_elapsed_ns.saturating_sub(e.timestamp_ns)),
                    value: format!("{}", e.value),
                    thread_id: e.tid,
                    source: None,
                })
                .collect(),
        }
    }
}
