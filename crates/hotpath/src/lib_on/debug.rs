//! Debug subsystem - value logging, debug logging, and gauges.

use crate::channels::{LOGS_LIMIT, START_TIME};
use crate::metrics_server::METRICS_SERVER_PORT;
use crossbeam_channel::unbounded;
#[cfg(not(feature = "hotpath-meta"))]
use crossbeam_channel::Sender as CbSender;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use crate::lib_on::{meta_rw_lock, MetaRwLock};

pub static DEBUG_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

static VAL_ID_REGISTRY: OnceLock<MetaRwLock<HashMap<String, u32>>> = OnceLock::new();
static GAUGE_ID_REGISTRY: OnceLock<MetaRwLock<HashMap<String, u32>>> = OnceLock::new();

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_or_create_val_id(key: &str) -> u32 {
    let registry = VAL_ID_REGISTRY.get_or_init(|| meta_rw_lock!("debug_val_ids", HashMap::new()));
    if let Some(&id) = registry.read().unwrap().get(key) {
        return id;
    }
    let mut write = registry.write().unwrap();
    *write
        .entry(key.to_string())
        .or_insert_with(|| DEBUG_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_or_create_gauge_id(key: &str) -> u32 {
    let registry =
        GAUGE_ID_REGISTRY.get_or_init(|| meta_rw_lock!("debug_gauge_ids", HashMap::new()));
    if let Some(&id) = registry.read().unwrap().get(key) {
        return id;
    }
    let mut write = registry.write().unwrap();
    *write
        .entry(key.to_string())
        .or_insert_with(|| DEBUG_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
}

use crate::instant::Instant;

pub mod dbg;
pub mod gauge;
pub mod val;

pub use dbg::log_dbg;
pub use gauge::GaugeHandle;
pub use val::ValHandle;

#[derive(Debug, Clone)]
pub(crate) struct DbgEntry {
    pub id: u32,
    pub source: &'static str,
    pub expression: &'static str,
    pub log_count: u64,
    pub logs: VecDeque<DbgLog>,
}

#[derive(Debug, Clone)]
pub(crate) struct DbgLog {
    pub index: u64,
    pub timestamp_ns: u64,
    pub value: String,
    pub tid: Option<u64>,
}

impl DbgEntry {
    fn new(id: u32, source: &'static str, expression: &'static str) -> Self {
        Self {
            id,
            source,
            expression,
            log_count: 0,
            logs: VecDeque::with_capacity(*LOGS_LIMIT),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ValEntry {
    pub id: u32,
    pub key: Arc<str>,
    pub log_count: u64,
    pub logs: VecDeque<ValLog>,
}

#[derive(Debug, Clone)]
pub(crate) struct ValLog {
    pub index: u64,
    pub timestamp_ns: u64,
    pub value: String,
    pub source: &'static str,
    pub tid: Option<u64>,
}

impl ValEntry {
    fn new(id: u32, key: Arc<str>) -> Self {
        Self {
            id,
            key,
            log_count: 0,
            logs: VecDeque::with_capacity(*LOGS_LIMIT),
        }
    }
}

#[derive(Debug)]
pub(crate) enum DebugEvent {
    Dbg {
        id: u32,
        source: &'static str,
        expression: &'static str,
        value: String,
        timestamp: Instant,
        tid: Option<u64>,
    },
    Val {
        id: u32,
        key: Arc<str>,
        source: &'static str,
        value: String,
        timestamp: Instant,
        tid: Option<u64>,
    },
    Gauge {
        id: u32,
        key: Arc<str>,
        source: &'static str,
        value: f64,
        timestamp: Instant,
        tid: Option<u64>,
    },
    GaugeInc {
        id: u32,
        key: Arc<str>,
        source: &'static str,
        delta: f64,
        timestamp: Instant,
        tid: Option<u64>,
    },
    GaugeDec {
        id: u32,
        key: Arc<str>,
        source: &'static str,
        delta: f64,
        timestamp: Instant,
        tid: Option<u64>,
    },
}

use gauge::{GaugeEntry, GaugeLog};

// `wrap` endpoint wrapper under `hotpath-meta` (instrumented), plain sender otherwise.
#[cfg(feature = "hotpath-meta")]
type DebugEventTx = hotpath_meta::wrap::crossbeam::Sender<DebugEvent>;
#[cfg(not(feature = "hotpath-meta"))]
type DebugEventTx = CbSender<DebugEvent>;

struct DebugState {
    event_tx: DebugEventTx,
    dbg: Arc<MetaRwLock<HashMap<u32, DbgEntry>>>,
    val: Arc<MetaRwLock<HashMap<u32, ValEntry>>>,
    gauge: Arc<MetaRwLock<HashMap<u32, GaugeEntry>>>,
}

static DEBUG_STATE: OnceLock<DebugState> = OnceLock::new();

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn init_debug_state() {
    DEBUG_STATE.get_or_init(|| {
        START_TIME.get_or_init(Instant::now);

        crate::metrics_server::start_metrics_server_once(*METRICS_SERVER_PORT);

        let (event_tx, event_rx) = unbounded::<DebugEvent>();
        #[cfg(feature = "hotpath-meta")]
        let (event_tx, event_rx) =
            hotpath_meta::channel!((event_tx, event_rx), label = "hp-dbg-events", log = true);
        let dbg = Arc::new(meta_rw_lock!(
            "debug_dbg_state",
            HashMap::<u32, DbgEntry>::new(),
        ));
        let val = Arc::new(meta_rw_lock!(
            "debug_val_state",
            HashMap::<u32, ValEntry>::new(),
        ));
        let gauge = Arc::new(meta_rw_lock!(
            "debug_gauge_state",
            HashMap::<u32, GaugeEntry>::new(),
        ));
        let dbg_clone = Arc::clone(&dbg);
        let val_clone = Arc::clone(&val);
        let gauge_clone = Arc::clone(&gauge);

        crate::lib_on::spawn_profiler_thread("hp-debug", move || {
            while let Ok(event) = event_rx.recv() {
                match event {
                    DebugEvent::Dbg { .. } => {
                        let mut stats = dbg_clone.write().unwrap();
                        process_dbg_event(&mut stats, event);
                    }
                    DebugEvent::Val { .. } => {
                        let mut stats = val_clone.write().unwrap();
                        process_val_event(&mut stats, event);
                    }
                    DebugEvent::Gauge { .. }
                    | DebugEvent::GaugeInc { .. }
                    | DebugEvent::GaugeDec { .. } => {
                        let mut stats = gauge_clone.write().unwrap();
                        process_gauge_event(&mut stats, event);
                    }
                }
            }
        })
        .expect("Failed to spawn debug event collector thread");

        DebugState {
            event_tx,
            dbg,
            val,
            gauge,
        }
    });
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn timestamp_nanos(timestamp: Instant) -> u64 {
    let start_time = START_TIME.get().copied().unwrap_or(timestamp);
    timestamp.duration_since(start_time).as_nanos() as u64
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn process_dbg_event(stats_map: &mut HashMap<u32, DbgEntry>, event: DebugEvent) {
    let DebugEvent::Dbg {
        id,
        source,
        expression,
        value,
        timestamp,
        tid,
    } = event
    else {
        return;
    };

    let stats = stats_map
        .entry(id)
        .or_insert_with(|| DbgEntry::new(id, source, expression));

    stats.log_count += 1;

    let entry = DbgLog {
        index: stats.log_count,
        timestamp_ns: timestamp_nanos(timestamp),
        value,
        tid,
    };

    let limit = *LOGS_LIMIT;
    if stats.logs.len() >= limit {
        stats.logs.pop_front();
    }
    stats.logs.push_back(entry);
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn process_val_event(stats_map: &mut HashMap<u32, ValEntry>, event: DebugEvent) {
    let DebugEvent::Val {
        id,
        key,
        source,
        value,
        timestamp,
        tid,
    } = event
    else {
        return;
    };

    let stats = stats_map
        .entry(id)
        .or_insert_with(|| ValEntry::new(id, key));

    stats.log_count += 1;

    let entry = ValLog {
        index: stats.log_count,
        timestamp_ns: timestamp_nanos(timestamp),
        value,
        source,
        tid,
    };

    let limit = *LOGS_LIMIT;
    if stats.logs.len() >= limit {
        stats.logs.pop_front();
    }
    stats.logs.push_back(entry);
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
fn process_gauge_event(stats_map: &mut HashMap<u32, GaugeEntry>, event: DebugEvent) {
    let (id, key, source, new_value, timestamp, tid) = match event {
        DebugEvent::Gauge {
            id,
            key,
            source,
            value,
            timestamp,
            tid,
        } => (id, key, source, value, timestamp, tid),
        DebugEvent::GaugeInc {
            id,
            key,
            source,
            delta,
            timestamp,
            tid,
        } => {
            let current = stats_map.get(&id).map(|s| s.current_value).unwrap_or(0.0);
            (id, key, source, current + delta, timestamp, tid)
        }
        DebugEvent::GaugeDec {
            id,
            key,
            source,
            delta,
            timestamp,
            tid,
        } => {
            let current = stats_map.get(&id).map(|s| s.current_value).unwrap_or(0.0);
            (id, key, source, current - delta, timestamp, tid)
        }
        _ => return,
    };

    let stats = stats_map
        .entry(id)
        .or_insert_with(|| GaugeEntry::new(id, key, source, new_value));

    stats.current_value = new_value;
    stats.min_value = stats.min_value.min(new_value);
    stats.max_value = stats.max_value.max(new_value);
    stats.update_count += 1;

    let entry = GaugeLog {
        index: stats.update_count,
        timestamp_ns: timestamp_nanos(timestamp),
        value: new_value,
        tid,
    };

    let limit = *LOGS_LIMIT;
    if stats.logs.len() >= limit {
        stats.logs.pop_front();
    }
    stats.logs.push_back(entry);
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn send_debug_event(event: DebugEvent) {
    if let Some(state) = DEBUG_STATE.get() {
        let _ = state.event_tx.send(event);
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_sorted_debug_dbg_entries() -> Vec<DbgEntry> {
    let mut stats: Vec<DbgEntry> = get_all_debug_dbg_entries().into_values().collect();
    stats.sort_by(|a, b| a.source.cmp(b.source).then(a.expression.cmp(b.expression)));
    stats
}

fn get_all_debug_dbg_entries() -> HashMap<u32, DbgEntry> {
    if let Some(state) = DEBUG_STATE.get() {
        state.dbg.read().unwrap().clone()
    } else {
        HashMap::new()
    }
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_sorted_debug_val_entries() -> Vec<ValEntry> {
    let mut stats: Vec<ValEntry> = get_all_debug_val_entries().into_values().collect();
    stats.sort_by(|a, b| a.key.cmp(&b.key));
    stats
}

fn get_all_debug_val_entries() -> HashMap<u32, ValEntry> {
    if let Some(state) = DEBUG_STATE.get() {
        state.val.read().unwrap().clone()
    } else {
        HashMap::new()
    }
}

pub(crate) fn get_debug_dbg_entries_by_id(id: u32) -> Option<DbgEntry> {
    DEBUG_STATE
        .get()
        .and_then(|state| state.dbg.read().ok())
        .and_then(|map| map.get(&id).cloned())
}

pub(crate) fn get_debug_val_entries_by_id(id: u32) -> Option<ValEntry> {
    DEBUG_STATE
        .get()
        .and_then(|state| state.val.read().ok())
        .and_then(|map| map.get(&id).cloned())
}

#[cfg_attr(feature = "hotpath-meta", hotpath_meta::measure(log = true))]
pub(crate) fn get_sorted_debug_gauge_entries() -> Vec<GaugeEntry> {
    let mut stats: Vec<GaugeEntry> = get_all_debug_gauge_entries().into_values().collect();
    stats.sort_by(|a, b| a.key.cmp(&b.key));
    stats
}

fn get_all_debug_gauge_entries() -> HashMap<u32, GaugeEntry> {
    if let Some(state) = DEBUG_STATE.get() {
        state.gauge.read().unwrap().clone()
    } else {
        HashMap::new()
    }
}

pub(crate) fn get_debug_gauge_entries_by_id(id: u32) -> Option<GaugeEntry> {
    DEBUG_STATE
        .get()
        .and_then(|state| state.gauge.read().ok())
        .and_then(|map| map.get(&id).cloned())
}
