use std::borrow::Borrow;
use std::collections::HashMap;
use std::io::Write;

use crate::table::{Cell, Table};

use crate::channels::{
    channel_to_json, compare_channel_entries, resolve_label, ChannelEntry, CHANNELS_STATE,
};
use crate::debug::{
    get_sorted_debug_dbg_entries, get_sorted_debug_gauge_entries, get_sorted_debug_val_entries,
};
use crate::futures::{compare_future_stats, FutureEntry, FUTURES_STATE};
use crate::http::{compare_http_entries, HttpEntry, HTTP_STATE};
use crate::io::{compare_io_entries, IoEntry, IoOpKind, IoOpStats, IO_STATE};
use crate::json::JsonDebugEntry;
use crate::json::{
    JsonChannelsList, JsonFutureEntry, JsonFuturesList, JsonHttpEntry, JsonHttpList, JsonIoEntry,
    JsonIoList, JsonIoOpStats, JsonMutexEntry, JsonMutexesList, JsonRwLockEntry, JsonRwLocksList,
    JsonServerAlloc, JsonServerEntry, JsonServerList, JsonSqlEntry, JsonSqlList, JsonStreamEntry,
    JsonStreamsList,
};
use crate::mutexes::{compare_mutex_entries, MutexEntry, MUTEXES_STATE};
use crate::output::{
    format_bytes, format_count_display, format_duration, format_percentile_header,
    format_percentile_key, format_rate, Precision,
};
use crate::output_on::{format_bytes_per_sec, format_throughput, write_section_header};
use crate::rw_locks::{compare_rw_lock_entries, RwLockEntry, RwLockKind, RW_LOCKS_STATE};
use crate::server::{compare_server_entries, ServerEntry, SERVER_STATE};
use crate::sql::{compare_sql_entries, SqlEntry, SQL_STATE};
use crate::streams::{compare_stream_stats, StreamStats, STREAMS_STATE};

/// `-` for entries with events but no measured duration (count-only sampling).
fn format_sampled_duration(
    precision: Precision,
    nanos: u64,
    sampled_count: u64,
    count: u64,
) -> String {
    if sampled_count == 0 && count > 0 {
        "-".to_string()
    } else {
        precision.duration(nanos)
    }
}

fn print_table(table: &Table, writer: &mut dyn Write) {
    let _ = table.print(writer, crate::output_on::use_colors());
}

/// Stops the worker and moves its entries out for the final report. Every
/// `shutdown_*` below takes the entries rather than cloning them: nothing
/// writes to the map after the completion signal, and the live metrics path
/// seeing an empty section only matters during process exit.
pub(crate) fn shutdown_channels() -> Vec<ChannelEntry> {
    crate::channels::stop_channel_events();
    CHANNELS_STATE
        .get()
        .and_then(|state| {
            if let Ok(mut guard) = state.shutdown_tx.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
            state
                .completion_rx
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .and_then(|rx| rx.recv().ok());
            state.inner.write().ok().map(|mut inner| {
                std::mem::take(&mut inner.stats)
                    .into_values()
                    .collect::<Vec<_>>()
            })
        })
        .map(|mut channels| {
            channels.sort_by(compare_channel_entries);
            channels
        })
        .unwrap_or_default()
}

pub(crate) fn report_channels_table(
    channels: &[ChannelEntry],
    total_count: usize,
    elapsed: std::time::Duration,
    writer: &mut dyn Write,
) {
    let now_ns = elapsed.as_nanos() as u64;
    if channels.is_empty() {
        return;
    }

    write_section_header(writer, "channels", "Channel throughput statistics.");

    let mut table = Table::new();
    table.add_row(vec![
        Cell::header("Channel"),
        Cell::header("Type"),
        Cell::header("Inst"),
        Cell::header("Sent"),
        Cell::header("Received"),
        Cell::header("Sent/s"),
        Cell::header("Recv/s"),
        Cell::header("Max queue"),
    ]);

    for channel_stats in channels {
        let label = resolve_label(
            channel_stats.source,
            channel_stats.label.as_deref(),
            Some(channel_stats.iter),
        );
        // `None` until the first message event.
        let max_queue = channel_stats
            .max_queue_size
            .map_or_else(|| "-".to_string(), |q| format_count_display(q as u64));
        table.add_row(vec![
            Cell::new(&label),
            Cell::new(&channel_stats.channel_type.to_string()),
            Cell::new(&format_count_display(channel_stats.instances)),
            Cell::new(&format_count_display(channel_stats.sent_count)),
            Cell::new(&format_count_display(channel_stats.received_count)),
            Cell::new(&format_rate(channel_stats.sent_per_sec(now_ns))),
            Cell::new(&format_rate(channel_stats.received_per_sec(now_ns))),
            Cell::new(&max_queue),
        ]);
    }

    if channels.len() < total_count {
        let _ = write!(writer, " ({}/{})", channels.len(), total_count);
    }
    let _ = writeln!(writer);
    print_table(&table, writer);
    let _ = writeln!(writer);
}

pub(crate) fn report_channel_latency_table(
    channels: &[ChannelEntry],
    percentiles: &[f64],
    writer: &mut dyn Write,
) {
    let rows: Vec<&ChannelEntry> = channels.iter().filter(|c| c.received_count > 0).collect();
    if rows.is_empty() {
        return;
    }

    write_section_header(
        writer,
        "channels latency",
        "Channel send->receive latency statistics.",
    );
    let _ = writeln!(writer);

    let mut header = vec![
        Cell::header("Channel"),
        Cell::header("Msgs"),
        Cell::header("Avg"),
    ];
    for &p in percentiles {
        header.push(Cell::header(&format_percentile_header(p)));
    }

    let mut table = Table::new();
    table.add_row(header);

    for channel in rows {
        let label = resolve_label(channel.source, channel.label.as_deref(), Some(channel.iter));
        let count_only = channel.delay_sampled_count == 0;
        let duration_cell = |nanos: u64| {
            if count_only {
                Cell::new("-")
            } else {
                Cell::new(&format_duration(nanos))
            }
        };
        let mut row = vec![
            Cell::new(&label),
            Cell::new(&format_count_display(channel.received_count)),
            duration_cell(channel.delay_avg_nanos()),
        ];
        for &p in percentiles {
            row.push(duration_cell(channel.delay_percentile_nanos(p)));
        }
        table.add_row(row);
    }

    print_table(&table, writer);
    let _ = writeln!(writer);
}

pub(crate) fn collect_channels_json(
    channels: &[impl Borrow<ChannelEntry>],
    limit: usize,
    elapsed: std::time::Duration,
    percentiles: &[f64],
    cloud: bool,
) -> JsonChannelsList {
    let current_elapsed_ns = elapsed.as_nanos() as u64;
    let total_count = channels.len();
    let channels = &channels[..apply_limit(total_count, limit)];
    JsonChannelsList {
        total_count,
        included_count: channels.len(),
        current_elapsed_ns,
        percentiles: percentiles.to_vec(),
        data: channels
            .iter()
            .map(|entry| channel_to_json(entry.borrow(), percentiles, current_elapsed_ns, cloud))
            .collect(),
    }
}

pub(crate) fn shutdown_rw_locks() -> Vec<RwLockEntry> {
    crate::lib_on::rw_locks::stop_rw_lock_events();
    RW_LOCKS_STATE
        .get()
        .and_then(|state| {
            if let Ok(mut guard) = state.shutdown_tx.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
            state
                .completion_rx
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .and_then(|rx| rx.recv().ok());
            state.inner.write().ok().map(|mut inner| {
                std::mem::take(&mut inner.stats)
                    .into_values()
                    .collect::<Vec<_>>()
            })
        })
        .map(|mut rw_locks| {
            rw_locks.sort_by(compare_rw_lock_entries);
            rw_locks
        })
        .unwrap_or_default()
}

pub(crate) fn report_rw_locks_table(
    rw_locks: &[RwLockEntry],
    total_count: usize,
    percentiles: &[f64],
    writer: &mut dyn Write,
) {
    if rw_locks.is_empty() {
        return;
    }

    write_section_header(writer, "rw_locks", "RwLock wait & acquire time statistics.");
    if rw_locks.len() < total_count {
        let _ = write!(writer, " ({}/{})", rw_locks.len(), total_count);
    }
    let _ = writeln!(writer);

    report_rw_locks_subtable(rw_locks, RwLockKind::Read, percentiles, writer);
    report_rw_locks_subtable(rw_locks, RwLockKind::Write, percentiles, writer);
}

fn report_rw_locks_subtable(
    rw_locks: &[RwLockEntry],
    kind: RwLockKind,
    percentiles: &[f64],
    writer: &mut dyn Write,
) {
    let rows: Vec<&RwLockEntry> = rw_locks.iter().filter(|l| l.count(kind) > 0).collect();
    if rows.is_empty() {
        return;
    }

    let count_label = match kind {
        RwLockKind::Read => "Reads",
        RwLockKind::Write => "Writes",
    };

    let mut header = vec![
        Cell::header("RwLock"),
        Cell::header(count_label),
        Cell::header("Wait avg"),
    ];
    for &p in percentiles {
        header.push(Cell::header(&format!(
            "Wait {}",
            format_percentile_header(p)
        )));
    }
    header.push(Cell::header("Acq avg"));
    for &p in percentiles {
        header.push(Cell::header(&format!(
            "Acq {}",
            format_percentile_header(p)
        )));
    }

    let mut table = Table::new();
    table.add_row(header);

    for rw_lock in rows {
        let label = resolve_label(rw_lock.source, rw_lock.label.as_deref(), Some(rw_lock.iter));
        let fmt = |nanos: u64| {
            format_sampled_duration(
                Precision::Display,
                nanos,
                rw_lock.sampled_count(kind),
                rw_lock.count(kind),
            )
        };
        let mut row = vec![
            Cell::new(&label),
            Cell::new(&format_count_display(rw_lock.count(kind))),
            Cell::new(&fmt(rw_lock.wait_avg_nanos(kind))),
        ];
        for &p in percentiles {
            row.push(Cell::new(&fmt(rw_lock.wait_percentile_nanos(kind, p))));
        }
        row.push(Cell::new(&fmt(rw_lock.acquire_avg_nanos(kind))));
        for &p in percentiles {
            row.push(Cell::new(&fmt(rw_lock.acquire_percentile_nanos(kind, p))));
        }
        table.add_row(row);
    }

    print_table(&table, writer);
    let _ = writeln!(writer);
}

fn rw_lock_to_json(rw_lock: &RwLockEntry, percentiles: &[f64], cloud: bool) -> JsonRwLockEntry {
    let precision = Precision::for_cloud(cloud);
    let label = resolve_label(rw_lock.source, rw_lock.label.as_deref(), Some(rw_lock.iter));

    let fmt = |kind: RwLockKind, nanos: u64| {
        format_sampled_duration(
            precision,
            nanos,
            rw_lock.sampled_count(kind),
            rw_lock.count(kind),
        )
    };
    let mut read_wait_percentiles = HashMap::new();
    let mut write_wait_percentiles = HashMap::new();
    let mut read_acquire_percentiles = HashMap::new();
    let mut write_acquire_percentiles = HashMap::new();
    for &p in percentiles {
        let key = format_percentile_key(p);
        read_wait_percentiles.insert(
            key.clone(),
            fmt(
                RwLockKind::Read,
                rw_lock.wait_percentile_nanos(RwLockKind::Read, p),
            ),
        );
        write_wait_percentiles.insert(
            key.clone(),
            fmt(
                RwLockKind::Write,
                rw_lock.wait_percentile_nanos(RwLockKind::Write, p),
            ),
        );
        read_acquire_percentiles.insert(
            key.clone(),
            fmt(
                RwLockKind::Read,
                rw_lock.acquire_percentile_nanos(RwLockKind::Read, p),
            ),
        );
        write_acquire_percentiles.insert(
            key,
            fmt(
                RwLockKind::Write,
                rw_lock.acquire_percentile_nanos(RwLockKind::Write, p),
            ),
        );
    }

    JsonRwLockEntry {
        id: rw_lock.id,
        source: rw_lock.source.to_string(),
        label,
        has_custom_label: rw_lock.label.is_some(),
        type_name: rw_lock.type_name.to_string(),
        read_count: rw_lock.read_count,
        write_count: rw_lock.write_count,
        read_sampled_count: rw_lock.read_sampled_count,
        write_sampled_count: rw_lock.write_sampled_count,
        read_wait_avg: fmt(RwLockKind::Read, rw_lock.wait_avg_nanos(RwLockKind::Read)),
        write_wait_avg: fmt(RwLockKind::Write, rw_lock.wait_avg_nanos(RwLockKind::Write)),
        read_acquire_avg: fmt(
            RwLockKind::Read,
            rw_lock.acquire_avg_nanos(RwLockKind::Read),
        ),
        write_acquire_avg: fmt(
            RwLockKind::Write,
            rw_lock.acquire_avg_nanos(RwLockKind::Write),
        ),
        read_wait_percentiles,
        write_wait_percentiles,
        read_acquire_percentiles,
        write_acquire_percentiles,
        location: crate::lib_on::locations::location_for_key(rw_lock.key),
        iter: rw_lock.iter,
    }
}

pub(crate) fn collect_rw_locks_json(
    rw_locks: &[impl Borrow<RwLockEntry>],
    limit: usize,
    elapsed: std::time::Duration,
    percentiles: &[f64],
    cloud: bool,
) -> JsonRwLocksList {
    let total_count = rw_locks.len();
    let rw_locks = &rw_locks[..apply_limit(total_count, limit)];
    JsonRwLocksList {
        total_count,
        included_count: rw_locks.len(),
        current_elapsed_ns: elapsed.as_nanos() as u64,
        percentiles: percentiles.to_vec(),
        data: rw_locks
            .iter()
            .map(|rw_lock| rw_lock_to_json(rw_lock.borrow(), percentiles, cloud))
            .collect(),
    }
}

pub(crate) fn shutdown_mutexes() -> Vec<MutexEntry> {
    crate::lib_on::mutexes::stop_mutex_events();
    MUTEXES_STATE
        .get()
        .and_then(|state| {
            if let Ok(mut guard) = state.shutdown_tx.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
            state
                .completion_rx
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .and_then(|rx| rx.recv().ok());
            state.inner.write().ok().map(|mut inner| {
                std::mem::take(&mut inner.stats)
                    .into_values()
                    .collect::<Vec<_>>()
            })
        })
        .map(|mut mutexes| {
            mutexes.sort_by(compare_mutex_entries);
            mutexes
        })
        .unwrap_or_default()
}

pub(crate) fn report_mutexes_table(
    mutexes: &[MutexEntry],
    total_count: usize,
    percentiles: &[f64],
    writer: &mut dyn Write,
) {
    let rows: Vec<&MutexEntry> = mutexes.iter().filter(|l| l.count > 0).collect();
    if rows.is_empty() {
        return;
    }

    write_section_header(writer, "mutexes", "Mutex wait & acquire time statistics.");
    if mutexes.len() < total_count {
        let _ = write!(writer, " ({}/{})", mutexes.len(), total_count);
    }
    let _ = writeln!(writer);

    let mut header = vec![
        Cell::header("Mutex"),
        Cell::header("Locks"),
        Cell::header("Wait avg"),
    ];
    for &p in percentiles {
        header.push(Cell::header(&format!(
            "Wait {}",
            format_percentile_header(p)
        )));
    }
    header.push(Cell::header("Acq avg"));
    for &p in percentiles {
        header.push(Cell::header(&format!(
            "Acq {}",
            format_percentile_header(p)
        )));
    }

    let mut table = Table::new();
    table.add_row(header);

    for mutex in rows {
        let label = resolve_label(mutex.source, mutex.label.as_deref(), Some(mutex.iter));
        let fmt = |nanos: u64| {
            format_sampled_duration(Precision::Display, nanos, mutex.sampled_count, mutex.count)
        };
        let mut row = vec![
            Cell::new(&label),
            Cell::new(&format_count_display(mutex.count)),
            Cell::new(&fmt(mutex.wait_avg_nanos())),
        ];
        for &p in percentiles {
            row.push(Cell::new(&fmt(mutex.wait_percentile_nanos(p))));
        }
        row.push(Cell::new(&fmt(mutex.acquire_avg_nanos())));
        for &p in percentiles {
            row.push(Cell::new(&fmt(mutex.acquire_percentile_nanos(p))));
        }
        table.add_row(row);
    }

    print_table(&table, writer);
    let _ = writeln!(writer);
}

fn mutex_to_json(mutex: &MutexEntry, percentiles: &[f64], cloud: bool) -> JsonMutexEntry {
    let precision = Precision::for_cloud(cloud);
    let label = resolve_label(mutex.source, mutex.label.as_deref(), Some(mutex.iter));

    let fmt =
        |nanos: u64| format_sampled_duration(precision, nanos, mutex.sampled_count, mutex.count);
    let mut wait_percentiles = HashMap::new();
    let mut acquire_percentiles = HashMap::new();
    for &p in percentiles {
        let key = format_percentile_key(p);
        wait_percentiles.insert(key.clone(), fmt(mutex.wait_percentile_nanos(p)));
        acquire_percentiles.insert(key, fmt(mutex.acquire_percentile_nanos(p)));
    }

    JsonMutexEntry {
        id: mutex.id,
        source: mutex.source.to_string(),
        label,
        has_custom_label: mutex.label.is_some(),
        type_name: mutex.type_name.to_string(),
        count: mutex.count,
        sampled_count: mutex.sampled_count,
        wait_avg: fmt(mutex.wait_avg_nanos()),
        acquire_avg: fmt(mutex.acquire_avg_nanos()),
        wait_percentiles,
        acquire_percentiles,
        location: crate::lib_on::locations::location_for_key(mutex.key),
        iter: mutex.iter,
    }
}

pub(crate) fn collect_mutexes_json(
    mutexes: &[impl Borrow<MutexEntry>],
    limit: usize,
    elapsed: std::time::Duration,
    percentiles: &[f64],
    cloud: bool,
) -> JsonMutexesList {
    let total_count = mutexes.len();
    let mutexes = &mutexes[..apply_limit(total_count, limit)];
    JsonMutexesList {
        total_count,
        included_count: mutexes.len(),
        current_elapsed_ns: elapsed.as_nanos() as u64,
        percentiles: percentiles.to_vec(),
        data: mutexes
            .iter()
            .map(|mutex| mutex_to_json(mutex.borrow(), percentiles, cloud))
            .collect(),
    }
}

pub(crate) fn shutdown_sql() -> Vec<SqlEntry> {
    crate::lib_on::sql::stop_sql_events();
    SQL_STATE
        .get()
        .and_then(|state| {
            if let Ok(mut guard) = state.shutdown_tx.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
            state
                .completion_rx
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .and_then(|rx| rx.recv().ok());
            state.inner.write().ok().map(|mut inner| {
                std::mem::take(&mut inner.stats)
                    .into_values()
                    .collect::<Vec<_>>()
            })
        })
        .map(|mut entries| {
            entries.sort_by(compare_sql_entries);
            entries
        })
        .unwrap_or_default()
}

const SQL_QUERY_DISPLAY_LEN: usize = 60;

fn truncate_query(query: &str) -> String {
    if query.chars().count() <= SQL_QUERY_DISPLAY_LEN {
        return query.to_string();
    }
    let truncated: String = query.chars().take(SQL_QUERY_DISPLAY_LEN - 3).collect();
    format!("{}...", truncated)
}

pub(crate) fn report_sql_table(
    entries: &[SqlEntry],
    total_count: usize,
    total_calls: u64,
    reference_total: u64,
    percentiles: &[f64],
    writer: &mut dyn Write,
) {
    if entries.is_empty() {
        return;
    }

    write_section_header(writer, "sql", "SQL query execution time statistics.");
    if entries.len() < total_count {
        let _ = write!(writer, " ({}/{})", entries.len(), total_count);
    }
    let _ = writeln!(writer);
    let _ = writeln!(writer, "Total calls: {}", total_calls);

    let show_route = entries.iter().any(|e| e.route.is_some());
    let mut header = vec![Cell::header("Query"), Cell::header("Source")];
    if show_route {
        header.push(Cell::header("Route"));
    }
    header.extend([Cell::header("Calls"), Cell::header("Avg")]);
    for &p in percentiles {
        header.push(Cell::header(&format_percentile_header(p)));
    }
    header.push(Cell::header("Total"));
    header.push(Cell::header("% Total"));

    let mut table = Table::new();
    table.add_row(header);

    for entry in entries {
        let mut row = vec![
            Cell::new(&truncate_query(&entry.query)),
            Cell::new(&format_source(entry.source)),
        ];
        if show_route {
            row.push(Cell::new(&format_route(entry.route)));
        }
        row.extend([
            Cell::new(&format_count_display(entry.count)),
            Cell::new(&format_duration(entry.avg_nanos())),
        ]);
        for &p in percentiles {
            row.push(Cell::new(&format_duration(entry.percentile_nanos(p))));
        }
        row.push(Cell::new(&format_duration(entry.total_nanos)));
        row.push(Cell::new(&format_sql_percent(
            entry.total_nanos,
            reference_total,
        )));
        table.add_row(row);
    }

    print_table(&table, writer);
    let _ = writeln!(writer);
}

fn format_source(source: Option<&'static str>) -> String {
    source.map_or_else(|| "-".to_string(), crate::output::shorten_function_name)
}

fn format_route(route: Option<&'static str>) -> String {
    route.unwrap_or("-").to_string()
}

fn format_sql_percent(total_nanos: u64, reference_total: u64) -> String {
    let percentage = if reference_total > 0 {
        (total_nanos as f64 / reference_total as f64) * 100.0
    } else {
        0.0
    };
    format!("{:.2}%", percentage)
}

fn sql_to_json(
    entry: &SqlEntry,
    reference_total: u64,
    percentiles: &[f64],
    cloud: bool,
) -> JsonSqlEntry {
    let precision = Precision::for_cloud(cloud);
    let mut percentile_map = HashMap::new();
    for &p in percentiles {
        percentile_map.insert(
            format_percentile_key(p),
            precision.duration(entry.percentile_nanos(p)),
        );
    }

    JsonSqlEntry {
        id: entry.id,
        query: entry.query.clone(),
        source: entry.source.map(String::from),
        route: entry.route.map(String::from),
        count: entry.count,
        avg: precision.duration(entry.avg_nanos()),
        total: precision.duration(entry.total_nanos),
        percent_total: format_sql_percent(entry.total_nanos, reference_total),
        percentiles: percentile_map,
        location: entry
            .source
            .and_then(crate::lib_on::locations::lookup_location),
    }
}

pub(crate) fn collect_sql_json(
    entries: &[impl Borrow<SqlEntry>],
    limit: usize,
    elapsed: std::time::Duration,
    percentiles: &[f64],
    cloud: bool,
) -> JsonSqlList {
    let reference_total: u64 = entries.iter().map(|e| e.borrow().total_nanos).sum();
    let total_calls: u64 = entries.iter().map(|e| e.borrow().count).sum();
    let total_count = entries.len();
    let entries = &entries[..apply_limit(total_count, limit)];
    JsonSqlList {
        total_count,
        included_count: entries.len(),
        current_elapsed_ns: elapsed.as_nanos() as u64,
        total_ns: reference_total,
        total_calls,
        percentiles: percentiles.to_vec(),
        data: entries
            .iter()
            .map(|entry| sql_to_json(entry.borrow(), reference_total, percentiles, cloud))
            .collect(),
    }
}

pub(crate) fn shutdown_http() -> Vec<HttpEntry> {
    crate::lib_on::http::stop_http_events();
    HTTP_STATE
        .get()
        .and_then(|state| {
            if let Ok(mut guard) = state.shutdown_tx.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
            state
                .completion_rx
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .and_then(|rx| rx.recv().ok());
            state.inner.write().ok().map(|mut inner| {
                std::mem::take(&mut inner.stats)
                    .into_values()
                    .collect::<Vec<_>>()
            })
        })
        .map(|mut entries| {
            entries.sort_by(compare_http_entries);
            entries
        })
        .unwrap_or_default()
}

pub(crate) fn report_http_table(
    entries: &[HttpEntry],
    total_count: usize,
    total_calls: u64,
    reference_total: u64,
    percentiles: &[f64],
    writer: &mut dyn Write,
) {
    if entries.is_empty() {
        return;
    }

    write_section_header(writer, "http", "HTTP request execution time statistics.");
    if entries.len() < total_count {
        let _ = write!(writer, " ({}/{})", entries.len(), total_count);
    }
    let _ = writeln!(writer);
    let _ = writeln!(writer, "Total calls: {}", total_calls);

    let show_route = entries.iter().any(|e| e.route.is_some());
    let mut header = vec![Cell::header("Endpoint"), Cell::header("Source")];
    if show_route {
        header.push(Cell::header("Route"));
    }
    header.extend([
        Cell::header("Calls"),
        Cell::header("Errors"),
        Cell::header("Avg"),
    ]);
    for &p in percentiles {
        header.push(Cell::header(&format_percentile_header(p)));
    }
    header.push(Cell::header("Total"));
    header.push(Cell::header("% Total"));

    let mut table = Table::new();
    table.add_row(header);

    for entry in entries {
        let mut row = vec![
            Cell::new(&truncate_query(&entry.endpoint)),
            Cell::new(&format_source(entry.source)),
        ];
        if show_route {
            row.push(Cell::new(&format_route(entry.route)));
        }
        row.extend([
            Cell::new(&format_count_display(entry.count)),
            Cell::new(&format_count_display(entry.error_count)),
            Cell::new(&format_duration(entry.avg_nanos())),
        ]);
        for &p in percentiles {
            row.push(Cell::new(&format_duration(entry.percentile_nanos(p))));
        }
        row.push(Cell::new(&format_duration(entry.total_nanos)));
        row.push(Cell::new(&format_sql_percent(
            entry.total_nanos,
            reference_total,
        )));
        table.add_row(row);
    }

    print_table(&table, writer);
    let _ = writeln!(writer);
}

fn http_to_json(
    entry: &HttpEntry,
    reference_total: u64,
    percentiles: &[f64],
    cloud: bool,
) -> JsonHttpEntry {
    let precision = Precision::for_cloud(cloud);
    let mut percentile_map = HashMap::new();
    for &p in percentiles {
        percentile_map.insert(
            format_percentile_key(p),
            precision.duration(entry.percentile_nanos(p)),
        );
    }

    JsonHttpEntry {
        id: entry.id,
        endpoint: entry.endpoint.clone(),
        source: entry.source.map(String::from),
        route: entry.route.map(String::from),
        count: entry.count,
        errors: entry.error_count,
        avg: precision.duration(entry.avg_nanos()),
        total: precision.duration(entry.total_nanos),
        percent_total: format_sql_percent(entry.total_nanos, reference_total),
        percentiles: percentile_map,
        location: entry
            .source
            .and_then(crate::lib_on::locations::lookup_location),
    }
}

pub(crate) fn collect_http_json(
    entries: &[impl Borrow<HttpEntry>],
    limit: usize,
    elapsed: std::time::Duration,
    percentiles: &[f64],
    cloud: bool,
) -> JsonHttpList {
    let reference_total: u64 = entries.iter().map(|e| e.borrow().total_nanos).sum();
    let total_calls: u64 = entries.iter().map(|e| e.borrow().count).sum();
    let total_count = entries.len();
    let entries = &entries[..apply_limit(total_count, limit)];
    JsonHttpList {
        total_count,
        included_count: entries.len(),
        current_elapsed_ns: elapsed.as_nanos() as u64,
        total_ns: reference_total,
        total_calls,
        percentiles: percentiles.to_vec(),
        data: entries
            .iter()
            .map(|entry| http_to_json(entry.borrow(), reference_total, percentiles, cloud))
            .collect(),
    }
}

pub(crate) fn shutdown_server() -> Vec<ServerEntry> {
    crate::lib_on::server::stop_server_events();
    SERVER_STATE
        .get()
        .and_then(|state| {
            if let Ok(mut guard) = state.shutdown_tx.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
            state
                .completion_rx
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .and_then(|rx| rx.recv().ok());
            state.inner.write().ok().map(|mut inner| {
                std::mem::take(&mut inner.stats)
                    .into_values()
                    .collect::<Vec<_>>()
            })
        })
        .map(|mut entries| {
            entries.sort_by(compare_server_entries);
            entries
        })
        .unwrap_or_default()
}

/// Which per-request columns the server table shows: each only when the
/// corresponding subsystem initialized, so an app without SQL profiling does
/// not get an all-`-` column.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ServerColumns {
    pub(crate) sql: bool,
    pub(crate) http: bool,
    /// Per-route memory (the second `server` sub-table and the JSON `alloc`
    /// object): only meaningful when the counting allocator is compiled in.
    pub(crate) alloc: bool,
}

impl ServerColumns {
    pub(crate) fn from_state() -> Self {
        Self {
            sql: SQL_STATE.get().is_some(),
            http: HTTP_STATE.get().is_some(),
            alloc: cfg!(feature = "hotpath-alloc-meta"),
        }
    }
}

fn format_per_request(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_string(), |v| format!("{v:.1}"))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn report_server_table(
    entries: &[ServerEntry],
    total_count: usize,
    total_calls: u64,
    reference_total: u64,
    reference_alloc_bytes: u64,
    percentiles: &[f64],
    columns: ServerColumns,
    writer: &mut dyn Write,
) {
    if entries.is_empty() {
        return;
    }

    write_section_header(
        writer,
        "server",
        "HTTP server response time statistics per route.",
    );
    if entries.len() < total_count {
        let _ = write!(writer, " ({}/{})", entries.len(), total_count);
    }
    let _ = writeln!(writer);
    let _ = writeln!(writer, "Total requests: {}", total_calls);

    let mut header = vec![
        Cell::header("Route"),
        Cell::header("Calls"),
        Cell::header("4xx"),
        Cell::header("5xx"),
    ];
    if columns.sql {
        header.push(Cell::header("SQL/req"));
    }
    if columns.http {
        header.push(Cell::header("HTTP/req"));
    }
    header.push(Cell::header("Avg"));
    for &p in percentiles {
        header.push(Cell::header(&format_percentile_header(p)));
    }
    header.push(Cell::header("Total"));
    header.push(Cell::header("% Total"));

    let mut table = Table::new();
    table.add_row(header);

    for entry in entries {
        let mut row = vec![
            Cell::new(&truncate_query(&entry.route)),
            Cell::new(&format_count_display(entry.count)),
            Cell::new(&format_count_display(entry.status_4xx)),
            Cell::new(&format_count_display(entry.status_5xx)),
        ];
        if columns.sql {
            row.push(Cell::new(&format_per_request(entry.sql_per_request())));
        }
        if columns.http {
            row.push(Cell::new(&format_per_request(entry.http_per_request())));
        }
        row.push(Cell::new(&format_duration(entry.avg_nanos())));
        for &p in percentiles {
            row.push(Cell::new(&format_duration(entry.percentile_nanos(p))));
        }
        row.push(Cell::new(&format_duration(entry.total_nanos)));
        row.push(Cell::new(&format_sql_percent(
            entry.total_nanos,
            reference_total,
        )));
        table.add_row(row);
    }

    print_table(&table, writer);
    let _ = writeln!(writer);

    if columns.alloc {
        report_server_alloc_subtable(entries, reference_alloc_bytes, percentiles, writer);
    }
}

/// Memory per route, stacked under the timing table in the same row order.
/// Rows without a route scope print `-` for every per-request value.
fn report_server_alloc_subtable(
    entries: &[ServerEntry],
    reference_total: u64,
    percentiles: &[f64],
    writer: &mut dyn Write,
) {
    let scoped = |entry: &ServerEntry, value: String| -> String {
        if entry.scoped_count > 0 {
            value
        } else {
            "-".to_string()
        }
    };

    let mut header = vec![
        Cell::header("Route"),
        Cell::header("Calls"),
        Cell::header("Allocs/req"),
        Cell::header("Avg"),
    ];
    for &p in percentiles {
        header.push(Cell::header(&format_percentile_header(p)));
    }
    header.push(Cell::header("Total"));
    header.push(Cell::header("% Total"));

    let mut table = Table::new();
    table.add_row(header);

    for entry in entries {
        let mut row = vec![
            Cell::new(&truncate_query(&entry.route)),
            Cell::new(&format_count_display(entry.count)),
            Cell::new(&format_per_request(entry.allocs_per_request())),
            Cell::new(&scoped(entry, format_bytes(entry.avg_bytes()))),
        ];
        for &p in percentiles {
            row.push(Cell::new(&scoped(
                entry,
                format_bytes(entry.percentile_bytes(p)),
            )));
        }
        row.push(Cell::new(&scoped(entry, format_bytes(entry.alloc_bytes))));
        row.push(Cell::new(&scoped(
            entry,
            format_sql_percent(entry.alloc_bytes, reference_total),
        )));
        table.add_row(row);
    }

    print_table(&table, writer);
    let _ = writeln!(writer);
}

fn server_alloc_to_json(
    entry: &ServerEntry,
    reference_total: u64,
    percentiles: &[f64],
    cloud: bool,
) -> JsonServerAlloc {
    let precision = Precision::for_cloud(cloud);
    let mut percentile_map = HashMap::new();
    for &p in percentiles {
        percentile_map.insert(
            format_percentile_key(p),
            precision.bytes(entry.percentile_bytes(p)),
        );
    }

    JsonServerAlloc {
        bytes_per_request: entry.bytes_per_request(),
        allocs_per_request: entry.allocs_per_request(),
        total_bytes: entry.alloc_bytes,
        avg: precision.bytes(entry.avg_bytes()),
        total: precision.bytes(entry.alloc_bytes),
        percent_total: format_sql_percent(entry.alloc_bytes, reference_total),
        percentiles: percentile_map,
    }
}

fn server_to_json(
    entry: &ServerEntry,
    reference_total: u64,
    reference_alloc_bytes: u64,
    percentiles: &[f64],
    columns: ServerColumns,
    cloud: bool,
) -> JsonServerEntry {
    let precision = Precision::for_cloud(cloud);
    let mut percentile_map = HashMap::new();
    for &p in percentiles {
        percentile_map.insert(
            format_percentile_key(p),
            precision.duration(entry.percentile_nanos(p)),
        );
    }

    JsonServerEntry {
        id: entry.id,
        route: entry.route.clone(),
        count: entry.count,
        status_4xx: entry.status_4xx,
        status_5xx: entry.status_5xx,
        sql_per_request: columns.sql.then(|| entry.sql_per_request()).flatten(),
        http_per_request: columns.http.then(|| entry.http_per_request()).flatten(),
        avg: precision.duration(entry.avg_nanos()),
        total: precision.duration(entry.total_nanos),
        percent_total: format_sql_percent(entry.total_nanos, reference_total),
        percentiles: percentile_map,
        alloc: columns
            .alloc
            .then(|| server_alloc_to_json(entry, reference_alloc_bytes, percentiles, cloud)),
    }
}

pub(crate) fn collect_server_json(
    entries: &[impl Borrow<ServerEntry>],
    limit: usize,
    elapsed: std::time::Duration,
    percentiles: &[f64],
    columns: ServerColumns,
    cloud: bool,
) -> JsonServerList {
    let reference_total: u64 = entries.iter().map(|e| e.borrow().total_nanos).sum();
    let total_alloc_bytes: u64 = entries.iter().map(|e| e.borrow().alloc_bytes).sum();
    let total_calls: u64 = entries.iter().map(|e| e.borrow().count).sum();
    let total_count = entries.len();
    let entries = &entries[..apply_limit(total_count, limit)];
    JsonServerList {
        total_count,
        included_count: entries.len(),
        current_elapsed_ns: elapsed.as_nanos() as u64,
        total_ns: reference_total,
        total_calls,
        total_alloc_bytes,
        percentiles: percentiles.to_vec(),
        data: entries
            .iter()
            .map(|entry| {
                server_to_json(
                    entry.borrow(),
                    reference_total,
                    total_alloc_bytes,
                    percentiles,
                    columns,
                    cloud,
                )
            })
            .collect(),
    }
}

pub(crate) fn shutdown_io() -> Vec<IoEntry> {
    crate::lib_on::io::stop_io_events();
    IO_STATE
        .get()
        .and_then(|state| {
            if let Ok(mut guard) = state.shutdown_tx.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
            state
                .completion_rx
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .and_then(|rx| rx.recv().ok());
            state.inner.write().ok().map(|mut inner| {
                std::mem::take(&mut inner.stats)
                    .into_values()
                    .collect::<Vec<_>>()
            })
        })
        .map(|mut entries| {
            entries.sort_by(compare_io_entries);
            entries
        })
        .unwrap_or_default()
}

pub(crate) fn report_io_table(
    entries: &[IoEntry],
    total_count: usize,
    percentiles: &[f64],
    writer: &mut dyn Write,
) {
    if entries.is_empty() {
        return;
    }

    write_section_header(writer, "io", "Byte-level I/O statistics.");
    if entries.len() < total_count {
        let _ = write!(writer, " ({}/{})", entries.len(), total_count);
    }
    let _ = writeln!(writer);

    report_io_subtable(entries, IoOpKind::Read, percentiles, writer);
    report_io_subtable(entries, IoOpKind::Write, percentiles, writer);
}

/// Reads and writes render as stacked sub-tables. The write sub-table carries
/// the flush count; shutdown operations appear only in JSON.
fn report_io_subtable(
    entries: &[IoEntry],
    kind: IoOpKind,
    percentiles: &[f64],
    writer: &mut dyn Write,
) {
    let rows: Vec<&IoEntry> = entries
        .iter()
        .filter(|e| match kind {
            IoOpKind::Read => e.read.count > 0 || e.read.errors > 0,
            _ => {
                e.write.count > 0
                    || e.flush.count > 0
                    || e.shutdown.count > 0
                    || e.write_side_errors() > 0
            }
        })
        .collect();
    if rows.is_empty() {
        return;
    }

    let count_label = match kind {
        IoOpKind::Read => "Reads",
        _ => "Writes",
    };

    let mut header = vec![
        Cell::header("Io"),
        Cell::header("Inst"),
        Cell::header(count_label),
        Cell::header("Bytes"),
        Cell::header("Rate"),
        Cell::header("Avg"),
    ];
    for &p in percentiles {
        header.push(Cell::header(&format_percentile_header(p)));
    }
    header.push(Cell::header("Total"));
    if kind == IoOpKind::Write {
        header.push(Cell::header("Flushes"));
    }
    header.push(Cell::header("Errors"));

    let mut table = Table::new();
    table.add_row(header);

    for entry in rows {
        let label = resolve_label(entry.source, entry.label.as_deref(), Some(entry.iter));
        let stats = entry.op(kind);
        let fmt = |nanos: u64| {
            format_sampled_duration(Precision::Display, nanos, stats.sampled_count, stats.count)
        };
        let mut row = vec![
            Cell::new(&label),
            Cell::new(&format_count_display(u64::from(entry.instances))),
            Cell::new(&format_count_display(stats.count)),
            Cell::new(&format_bytes(stats.bytes)),
            Cell::new(&format_throughput(stats.throughput_bytes_per_sec())),
            Cell::new(&fmt(stats.avg_nanos())),
        ];
        for &p in percentiles {
            row.push(Cell::new(&fmt(stats.percentile_nanos(p))));
        }
        row.push(Cell::new(&fmt(stats.total_nanos)));
        let errors = if kind == IoOpKind::Write {
            row.push(Cell::new(&format_count_display(entry.flush.count)));
            entry.write_side_errors()
        } else {
            stats.errors
        };
        row.push(Cell::new(&format_count_display(errors)));
        table.add_row(row);
    }

    print_table(&table, writer);
    let _ = writeln!(writer);
}

fn io_op_stats_to_json(stats: &IoOpStats, percentiles: &[f64], cloud: bool) -> JsonIoOpStats {
    let precision = Precision::for_cloud(cloud);
    let fmt =
        |nanos: u64| format_sampled_duration(precision, nanos, stats.sampled_count, stats.count);
    let mut percentile_map = HashMap::new();
    for &p in percentiles {
        percentile_map.insert(format_percentile_key(p), fmt(stats.percentile_nanos(p)));
    }

    JsonIoOpStats {
        count: stats.count,
        sampled_count: stats.sampled_count,
        bytes: stats.bytes,
        sampled_bytes: stats.sampled_bytes,
        errors: stats.errors,
        avg: fmt(stats.avg_nanos()),
        bytes_per_sec: stats.throughput_bytes_per_sec().map(format_bytes_per_sec),
        total_ns: stats.total_nanos,
        percentiles: percentile_map,
    }
}

fn io_to_json(entry: &IoEntry, percentiles: &[f64], cloud: bool) -> JsonIoEntry {
    let label = resolve_label(entry.source, entry.label.as_deref(), Some(entry.iter));

    JsonIoEntry {
        id: entry.id,
        source: entry.source.to_string(),
        label,
        has_custom_label: entry.label.is_some(),
        type_name: entry.type_name.to_string(),
        read: io_op_stats_to_json(&entry.read, percentiles, cloud),
        write: io_op_stats_to_json(&entry.write, percentiles, cloud),
        flush: io_op_stats_to_json(&entry.flush, percentiles, cloud),
        shutdown: io_op_stats_to_json(&entry.shutdown, percentiles, cloud),
        instances: entry.instances,
        location: crate::lib_on::locations::location_for_key(entry.key),
        iter: entry.iter,
    }
}

pub(crate) fn collect_io_json(
    entries: &[impl Borrow<IoEntry>],
    limit: usize,
    elapsed: std::time::Duration,
    percentiles: &[f64],
    cloud: bool,
) -> JsonIoList {
    let total_count = entries.len();
    let entries = &entries[..apply_limit(total_count, limit)];
    JsonIoList {
        total_count,
        included_count: entries.len(),
        current_elapsed_ns: elapsed.as_nanos() as u64,
        percentiles: percentiles.to_vec(),
        data: entries
            .iter()
            .map(|entry| io_to_json(entry.borrow(), percentiles, cloud))
            .collect(),
    }
}

pub(crate) fn shutdown_streams() -> Vec<StreamStats> {
    crate::streams::stop_stream_events();
    STREAMS_STATE
        .get()
        .and_then(|state| {
            if let Ok(mut guard) = state.shutdown_tx.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
            state
                .completion_rx
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .and_then(|rx| rx.recv().ok());
            state.inner.write().ok().map(|mut inner| {
                std::mem::take(&mut inner.stats)
                    .into_values()
                    .collect::<Vec<_>>()
            })
        })
        .map(|mut streams| {
            streams.sort_by(compare_stream_stats);
            streams
        })
        .unwrap_or_default()
}

pub(crate) fn report_streams_table(
    streams: &[StreamStats],
    total_count: usize,
    writer: &mut dyn Write,
) {
    if streams.is_empty() {
        return;
    }

    write_section_header(writer, "streams", "Stream yield statistics.");

    let mut table = Table::new();
    table.add_row(vec![
        Cell::header("Stream"),
        Cell::header("Inst"),
        Cell::header("State"),
        Cell::header("Yielded"),
    ]);

    for stream_stats in streams {
        let label = resolve_label(
            stream_stats.source,
            stream_stats.label.as_deref(),
            Some(stream_stats.iter),
        );
        table.add_row(vec![
            Cell::new(&label),
            Cell::new(&format_count_display(stream_stats.instances)),
            Cell::new(
                stream_stats
                    .display_state()
                    .map_or("-", |state| state.as_str()),
            ),
            Cell::new(&format_count_display(stream_stats.items_yielded)),
        ]);
    }

    if streams.len() < total_count {
        let _ = write!(writer, " ({}/{})", streams.len(), total_count);
    }
    let _ = writeln!(writer);
    print_table(&table, writer);
    let _ = writeln!(writer);
}

pub(crate) fn collect_streams_json(
    streams: &[StreamStats],
    limit: usize,
    elapsed: std::time::Duration,
) -> JsonStreamsList {
    let total_count = streams.len();
    let streams = &streams[..apply_limit(total_count, limit)];
    JsonStreamsList {
        total_count,
        included_count: streams.len(),
        current_elapsed_ns: elapsed.as_nanos() as u64,
        data: streams.iter().map(JsonStreamEntry::from).collect(),
    }
}

pub(crate) fn shutdown_futures() -> Vec<FutureEntry> {
    crate::lib_on::futures::stop_future_events();
    FUTURES_STATE
        .get()
        .and_then(|state| {
            if let Ok(mut guard) = state.shutdown_tx.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
            state
                .completion_rx
                .lock()
                .ok()
                .and_then(|mut guard| guard.take())
                .and_then(|rx| rx.recv().ok());
            state.inner.write().ok().map(|mut inner| {
                std::mem::take(&mut inner.stats)
                    .into_values()
                    .collect::<Vec<_>>()
            })
        })
        .map(|mut futures| {
            futures.sort_by(compare_future_stats);
            futures
        })
        .unwrap_or_default()
}

pub(crate) fn report_futures_table(
    futures: &[FutureEntry],
    total_count: usize,
    writer: &mut dyn Write,
) {
    if futures.is_empty() {
        return;
    }

    write_section_header(writer, "futures", "Future poll and lifecycle statistics.");

    let mut table = Table::new();
    table.add_row(vec![
        Cell::header("Future"),
        Cell::header("Calls"),
        Cell::header("Polls"),
        Cell::header("Avg Poll"),
        Cell::header("Total Poll"),
        Cell::header("Avg Alloc"),
        Cell::header("Total Alloc"),
    ]);

    for future_stats in futures {
        let label = resolve_label(
            crate::futures::display_source(future_stats.source),
            future_stats.label.as_deref(),
            None,
        );
        let total_calls = future_stats.logs_count;
        let total_polls = future_stats.total_polls();
        let total_poll_dur = future_stats.display_total_poll_duration_ns();
        let total_alloc_bytes_across_polls = future_stats.total_poll_alloc_bytes();
        let avg_poll = match future_stats.avg_poll_duration_ns() {
            Some(avg) => format_duration(avg),
            None => "-".to_string(),
        };
        let avg_alloc_per_call = match total_alloc_bytes_across_polls {
            Some(total_alloc_bytes) if total_calls > 0 => {
                format_bytes(total_alloc_bytes / total_calls)
            }
            _ => "-".to_string(),
        };
        let total_alloc = total_alloc_bytes_across_polls
            .map(format_bytes)
            .unwrap_or_else(|| "-".to_string());
        let total_poll_dur = if future_stats.sampled_polls == 0 && total_polls > 0 {
            "-".to_string()
        } else {
            format_duration(total_poll_dur)
        };
        table.add_row(vec![
            Cell::new(&label),
            Cell::new(&format_count_display(total_calls)),
            Cell::new(&format_count_display(total_polls)),
            Cell::new(&avg_poll),
            Cell::new(&total_poll_dur),
            Cell::new(&avg_alloc_per_call),
            Cell::new(&total_alloc),
        ]);
    }

    if futures.len() < total_count {
        let _ = write!(writer, " ({}/{})", futures.len(), total_count);
    }
    let _ = writeln!(writer);
    print_table(&table, writer);
    let _ = writeln!(writer);
}

pub(crate) fn collect_futures_json(
    futures: &[FutureEntry],
    limit: usize,
    elapsed: std::time::Duration,
) -> JsonFuturesList {
    let total_count = futures.len();
    let futures = &futures[..apply_limit(total_count, limit)];
    JsonFuturesList {
        total_count,
        included_count: futures.len(),
        current_elapsed_ns: elapsed.as_nanos() as u64,
        data: futures.iter().map(JsonFutureEntry::from).collect(),
    }
}

#[cfg(feature = "threads")]
pub(crate) fn report_threads_table(writer: &mut dyn Write, limit: usize) {
    let mut threads_json = crate::threads::get_threads_json(Precision::Display);

    if threads_json.data.is_empty() {
        return;
    }

    let total_count = threads_json.data.len();
    if limit > 0 && limit < total_count {
        threads_json.data.truncate(limit);
    }

    write_section_header(writer, "threads", "Thread CPU and memory statistics.");

    let has_alloc = threads_json.data.iter().any(|t| t.alloc_bytes.is_some());

    let mut header = vec![
        Cell::header("Thread"),
        Cell::header("Max%"),
        Cell::header("Avg%"),
    ];
    if has_alloc {
        header.push(Cell::header("Alloc"));
        header.push(Cell::header("Dealloc"));
        header.push(Cell::header("Diff"));
    }

    let mut table = Table::new();
    table.add_row(header);

    for thread in &threads_json.data {
        let cpu_pct_max = thread.cpu_percent_max.as_deref().unwrap_or("-");
        let cpu_pct_avg = thread.cpu_percent_avg.as_deref().unwrap_or("-");
        let mut row = vec![
            Cell::new(&thread.name),
            Cell::new(cpu_pct_max),
            Cell::new(cpu_pct_avg),
        ];
        if has_alloc {
            row.push(Cell::new(thread.alloc_bytes.as_deref().unwrap_or("-")));
            row.push(Cell::new(thread.dealloc_bytes.as_deref().unwrap_or("-")));
            row.push(Cell::new(thread.mem_diff.as_deref().unwrap_or("-")));
        }
        table.add_row(row);
    }

    // The report is printed once at exit, so the maximum describes the run
    // while a snapshot taken at that instant would not.
    let mut info_parts = Vec::new();
    if let Some(max_rss) = &threads_json.rss_bytes_max {
        info_parts.push(format!("Max RSS: {}", max_rss));
    }
    if let Some(alloc) = &threads_json.total_alloc_bytes {
        info_parts.push(format!("Alloc: {}", alloc));
    }
    if let Some(dealloc) = &threads_json.total_dealloc_bytes {
        info_parts.push(format!("Dealloc: {}", dealloc));
    }
    if let Some(diff) = &threads_json.alloc_dealloc_diff {
        info_parts.push(format!("Diff: {}", diff));
    }

    let displayed = threads_json.data.len();
    if displayed < total_count {
        info_parts.push(format!("{}/{}", displayed, total_count));
    }

    if !info_parts.is_empty() {
        let _ = write!(writer, " ({})", info_parts.join(", "));
    }
    let _ = writeln!(writer);
    print_table(&table, writer);
    let _ = writeln!(writer);
}

#[cfg(feature = "threads")]
pub(crate) fn collect_threads_json(limit: usize, cloud: bool) -> crate::json::JsonThreadsList {
    let mut json = crate::threads::get_threads_json(Precision::for_cloud(cloud));
    json.data.truncate(apply_limit(json.data.len(), limit));
    json.included_count = json.data.len();
    json
}

pub(crate) fn apply_limit(len: usize, limit: usize) -> usize {
    if limit > 0 && limit < len {
        limit
    } else {
        len
    }
}

pub(crate) fn has_debug_entries() -> bool {
    !get_sorted_debug_dbg_entries().is_empty()
        || !get_sorted_debug_val_entries().is_empty()
        || !get_sorted_debug_gauge_entries().is_empty()
}

pub(crate) fn report_debug_table(writer: &mut dyn Write) {
    let dbg_entries = get_sorted_debug_dbg_entries();
    let val_entries = get_sorted_debug_val_entries();
    let gauge_entries = get_sorted_debug_gauge_entries();

    if dbg_entries.is_empty() && val_entries.is_empty() && gauge_entries.is_empty() {
        return;
    }

    write_section_header(writer, "debug", "Debug last values (dbg!, val!, gauge!).");

    let header = vec![
        Cell::header("Type"),
        Cell::header("Key/Expr"),
        Cell::header("Value"),
        Cell::header("Updates"),
        Cell::header("Source"),
    ];

    let mut table = Table::new();
    table.add_row(header);

    let mut entries: Vec<JsonDebugEntry> = Vec::new();
    entries.extend(dbg_entries.iter().map(JsonDebugEntry::from));
    entries.extend(val_entries.iter().map(JsonDebugEntry::from));
    entries.extend(gauge_entries.iter().map(JsonDebugEntry::from));

    for entry in &entries {
        let value = entry.last_value.as_deref().unwrap_or("-");
        table.add_row(vec![
            Cell::new(entry.entry_type.as_str()),
            Cell::new(&entry.expression),
            Cell::new(value),
            Cell::new(&format_count_display(entry.log_count)),
            Cell::new(&entry.source_display),
        ]);
    }

    let _ = writeln!(writer);
    print_table(&table, writer);
    let _ = writeln!(writer);
}

pub(crate) fn collect_debug_json(
    elapsed: std::time::Duration,
    limit: usize,
) -> crate::json::JsonDebugList {
    let mut entries: Vec<JsonDebugEntry> = Vec::new();
    entries.extend(
        get_sorted_debug_dbg_entries()
            .iter()
            .map(JsonDebugEntry::from),
    );
    entries.extend(
        get_sorted_debug_val_entries()
            .iter()
            .map(JsonDebugEntry::from),
    );
    entries.extend(
        get_sorted_debug_gauge_entries()
            .iter()
            .map(JsonDebugEntry::from),
    );

    let total_count = entries.len();
    entries.truncate(apply_limit(total_count, limit));
    crate::json::JsonDebugList {
        current_elapsed_ns: elapsed.as_nanos() as u64,
        total_count,
        included_count: entries.len(),
        entries,
    }
}

#[cfg(test)]
mod tests {
    use crate::lib_on::report::format_per_request;

    #[test]
    fn per_request_formatting() {
        assert_eq!(format_per_request(None), "-");
        assert_eq!(format_per_request(Some(2.0)), "2.0");
        assert_eq!(format_per_request(Some(1.25)), "1.2");
    }
}
