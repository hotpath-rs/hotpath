use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::dev_logging::warn;
use crate::table::{Cell, Color, Table};

use crate::json::{
    CpuSnapshotStatus, JsonFunctionCpuEntry, JsonFunctionsCpuEnvelope, JsonFunctionsCpuList,
};
use crate::output::{format_count_display, format_duration, shorten_function_name};

#[allow(dead_code)]
pub(crate) mod autospawn;
pub(crate) mod json;
pub(crate) mod samply;

pub(crate) static CPU_INCLUSIVE: LazyLock<bool> =
    LazyLock::new(|| crate::shared::env_flag("HOTPATH_META_CPU_INCLUSIVE"));

#[derive(Debug, Clone)]
pub(crate) struct CpuFunctionStats {
    pub(crate) name: &'static str,
    pub(crate) id: u32,
    pub(crate) samples: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct CpuReport {
    pub(crate) total_samples: u64,
    pub(crate) attributed_samples: u64,
    pub(crate) caller_name: &'static str,
    pub(crate) stats: Vec<CpuFunctionStats>,
    pub(crate) profile_path: String,
}

pub(crate) fn build_cpu_report_from_path(
    caller_name: &'static str,
    path: &Path,
) -> Option<CpuReport> {
    samply::build_cpu_report_from_path(caller_name, path)
}

struct SnapshotState {
    status: CpuSnapshotStatus,
    report: Option<JsonFunctionsCpuList>,
    captured_at_ms: Option<u64>,
    capture_duration_ms: Option<u64>,
    error: Option<String>,
}

impl SnapshotState {
    const fn new() -> Self {
        Self {
            status: CpuSnapshotStatus::Idle,
            report: None,
            captured_at_ms: None,
            capture_duration_ms: None,
            error: None,
        }
    }
}

static SNAPSHOT_STATE: RwLock<SnapshotState> = RwLock::new(SnapshotState::new());
static SNAPSHOT_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

pub(crate) fn try_spawn_snapshot() -> bool {
    if SNAPSHOT_IN_PROGRESS
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }

    if let Ok(mut state) = SNAPSHOT_STATE.write() {
        state.status = CpuSnapshotStatus::Capturing;
        state.error = None;
    }

    let spawn_result = std::thread::Builder::new()
        .name("hp-cpu-snapshot".into())
        .spawn(|| {
            let _suspend = crate::lib_on::SuspendAllocTracking::new();
            run_snapshot();
            autospawn::start();
            SNAPSHOT_IN_PROGRESS.store(false, Ordering::Release);
        });
    if spawn_result.is_err() {
        SNAPSHOT_IN_PROGRESS.store(false, Ordering::Release);
        set_snapshot_error("failed to spawn snapshot thread");
        return false;
    }
    true
}

fn run_snapshot() {
    let started = Instant::now();
    let path = match autospawn::stop() {
        Ok(p) => p,
        Err(e) => {
            set_snapshot_error(&e);
            return;
        }
    };

    let caller_name = "snapshot";
    let report = match build_cpu_report_from_path(caller_name, &path) {
        Some(r) => r,
        None => {
            set_snapshot_error("failed to parse samply profile");
            return;
        }
    };

    let elapsed_ns = crate::lib_on::START_TIME
        .get()
        .map(|s| s.elapsed().as_nanos() as u64)
        .unwrap_or(0);
    let elapsed = std::time::Duration::from_nanos(elapsed_ns);
    let cpu_json = build_cpu_json(&report, elapsed, elapsed_ns, 0);

    if let Ok(mut state) = SNAPSHOT_STATE.write() {
        state.status = CpuSnapshotStatus::Ready;
        state.report = Some(cpu_json);
        state.captured_at_ms = current_unix_ms();
        state.capture_duration_ms = Some(started.elapsed().as_millis() as u64);
        state.error = None;
    }
}

fn set_snapshot_error(msg: &str) {
    warn!("cpu report: snapshot failed: {msg}");
    if let Ok(mut state) = SNAPSHOT_STATE.write() {
        state.status = CpuSnapshotStatus::Error;
        state.error = Some(msg.to_string());
    }
}

fn current_unix_ms() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

pub(crate) fn get_cpu_envelope() -> JsonFunctionsCpuEnvelope {
    let session = autospawn::current_session();
    let (current_session_id, current_session_path) = match session {
        Some(info) => (
            Some(info.session_id),
            Some(info.session_dir.to_string_lossy().into_owned()),
        ),
        None => (None, None),
    };

    let snapshot = SNAPSHOT_STATE.read();
    let (status, report, captured_at_ms, capture_duration_ms, error) = match snapshot {
        Ok(state) => (
            state.status,
            state.report.clone(),
            state.captured_at_ms,
            state.capture_duration_ms,
            state.error.clone(),
        ),
        Err(_) => (CpuSnapshotStatus::Idle, None, None, None, None),
    };

    JsonFunctionsCpuEnvelope {
        status,
        captured_at_ms,
        capture_duration_ms,
        error,
        report,
        current_session_id,
        current_session_path,
    }
}

fn format_percent(numer: u64, denom: u64) -> String {
    if denom == 0 {
        "0.00%".to_string()
    } else {
        format!("{:.2}%", (numer as f64 / denom as f64) * 100.0)
    }
}

pub(crate) fn build_cpu_json(
    report: &CpuReport,
    total_elapsed: std::time::Duration,
    current_elapsed_ns: u64,
    limit: usize,
) -> JsonFunctionsCpuList {
    let (wrapper_stats, inner_stats): (Vec<_>, Vec<_>) = report
        .stats
        .iter()
        .partition(|s| s.name == report.caller_name);

    let total_inner = inner_stats.len();
    let displayed_inner = if limit > 0 && limit < total_inner {
        limit
    } else {
        total_inner
    };

    let to_entry = |s: &CpuFunctionStats| JsonFunctionCpuEntry {
        id: s.id,
        name: s.name.to_string(),
        samples: s.samples,
        percent: format_percent(s.samples, report.total_samples),
    };

    let mut entries: Vec<JsonFunctionCpuEntry> =
        wrapper_stats.iter().map(|s| to_entry(s)).collect();
    entries.extend(
        inner_stats
            .iter()
            .take(displayed_inner)
            .map(|s| to_entry(s)),
    );

    let total_count = total_inner + wrapper_stats.len();
    let included_count = displayed_inner + wrapper_stats.len();

    let description = if *CPU_INCLUSIVE {
        "CPU sampling attribution per function (inclusive).".to_string()
    } else {
        "CPU sampling attribution per function (exclusive).".to_string()
    };

    JsonFunctionsCpuList {
        time_elapsed: format_duration(total_elapsed.as_nanos() as u64),
        total_elapsed_ns: current_elapsed_ns,
        total_samples: report.total_samples,
        attributed_samples: report.attributed_samples,
        description,
        caller_name: report.caller_name.to_string(),
        data: entries,
        profile_path: report.profile_path.clone(),
        included_count,
        total_count,
    }
}

fn print_table<W: Write>(table: &Table, writer: &mut W) {
    let _ = table.print(writer, crate::output_on::use_colors());
}

pub(crate) fn report_functions_cpu_table<W: Write>(writer: &mut W, list: &JsonFunctionsCpuList) {
    let mut info = format!("{} total samples", list.total_samples);
    if list.included_count < list.total_count {
        info.push_str(&format!(", {}/{}", list.included_count, list.total_count));
    }
    let _ = writeln!(writer, "cpu - {} ({})", list.description, info);

    if list.data.is_empty() {
        let _ = writeln!(writer, "no samples attributed to instrumented functions");
    } else {
        let mut table = Table::new();
        table.add_row(vec![
            Cell::header("Function"),
            Cell::header("Samples"),
            Cell::header("% Total"),
        ]);

        for entry in &list.data {
            let short_name = shorten_function_name(&entry.name);
            table.add_row(vec![
                Cell::new(&short_name),
                Cell::new(&format_count_display(entry.samples)),
                Cell::new(&entry.percent),
            ]);
        }
        print_table(&table, writer);
    }

    let _ = writeln!(
        writer,
        "{}",
        crate::output_on::cyan(&format!("samply load {}", list.profile_path))
    );
    let _ = writeln!(writer);
}

pub(crate) fn report_functions_cpu_error_table<W: Write>(writer: &mut W, message: &str) {
    let mut table = Table::new();
    table.add_row(vec![Cell::new("Error").bold().color(Color::Red)]);
    table.add_row(vec![Cell::new(message)]);

    let _ = writeln!(writer, "cpu - report unavailable");
    print_table(&table, writer);
    let _ = writeln!(writer);
}
