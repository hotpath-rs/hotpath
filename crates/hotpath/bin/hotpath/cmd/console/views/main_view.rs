use crate::cmd::console::app::{
    App, DataFlowFocus, DataFlowLogs, DataFlowSubTab, DebugFocus, FunctionsFocus, FunctionsSubTab,
    IoFocus, IoSubTab, SelectedTab, SubTabHit,
};
use crate::cmd::console::views::data_flow::{inspect as data_flow_inspect, logs as data_flow_logs};
use crate::cmd::console::views::debug::{inspect as debug_inspect, logs as debug_logs};
use crate::cmd::console::views::functions_memory::{
    inspect as memory_inspect, logs as memory_logs,
};
use crate::cmd::console::views::functions_timing::{
    inspect as timing_inspect, logs as timing_logs,
};
use crate::cmd::console::views::io::{inspect as io_inspect, logs as io_logs};
use crate::cmd::console::views::{
    bottom_bar, data_flow, debug, functions_cpu, functions_memory, functions_timing, runtime,
    threads, top_bar,
};
use crate::cmd::console::widgets::formatters::truncate_message;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Paragraph, Tabs},
    Frame,
};

#[hotpath::measure]
pub(crate) fn render_ui(frame: &mut Frame, app: &mut App) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Tabs
            Constraint::Length(1), // Subtabs (Functions only)
            Constraint::Length(3), // Status bar
            Constraint::Min(0),    // Main content area
            Constraint::Length(3), // Help bar
        ])
        .split(frame.area());

    let has_data = match app.selected_tab {
        SelectedTab::Functions => match app.functions_sub_tab {
            FunctionsSubTab::Timing => !app.timing_functions.data.is_empty(),
            FunctionsSubTab::Memory => !app.memory_functions.data.is_empty(),
            FunctionsSubTab::Cpu => app
                .cpu_envelope
                .as_ref()
                .and_then(|e| e.report.as_ref())
                .is_some_and(|r| !r.data.is_empty()),
        },
        SelectedTab::DataFlow => app.data_flow_entries_len() > 0,
        SelectedTab::Io => app.io_entries_len() > 0,
        SelectedTab::Threads => !app.threads.data.is_empty(),
        SelectedTab::Debug => !app.debug_stats.is_empty(),
        SelectedTab::Runtime => app.tokio_runtime.is_some(),
    };

    app.tab_hit_areas.clear();
    app.sub_tab_hit_areas.clear();

    render_tabs(frame, main_chunks[0], app);

    if app.selected_tab == SelectedTab::Functions {
        render_functions_subtabs(frame, main_chunks[1], app);
    } else if app.selected_tab == SelectedTab::DataFlow {
        render_data_flow_subtabs(frame, main_chunks[1], app);
    } else if app.selected_tab == SelectedTab::Io {
        render_io_subtabs(frame, main_chunks[1], app);
    }

    top_bar::render_status_bar(
        frame,
        main_chunks[2],
        app.paused,
        app.last_successful_fetch,
        app.error_message.is_some(),
        has_data,
        app.program_uptime.as_deref(),
        app.program_pid,
    );

    match app.selected_tab {
        SelectedTab::Functions => {
            render_functions_view(frame, app, main_chunks[3]);
        }
        SelectedTab::DataFlow => {
            render_data_flow_view(frame, app, main_chunks[3]);
        }
        SelectedTab::Io => {
            render_io_view(frame, app, main_chunks[3]);
        }
        SelectedTab::Threads => {
            render_threads_view(frame, app, main_chunks[3]);
        }
        SelectedTab::Debug => {
            render_debug_view(frame, app, main_chunks[3]);
        }
        SelectedTab::Runtime => {
            render_runtime_view(frame, app, main_chunks[3]);
        }
    }

    bottom_bar::render_help_bar(
        frame,
        main_chunks[4],
        app.selected_tab,
        app.data_flow_focus,
        app.functions_focus,
        app.debug_focus,
        app.io_focus,
    );
}

#[hotpath::measure]
fn render_functions_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let content_area = area;

    match app.functions_sub_tab {
        FunctionsSubTab::Timing => {
            if app.show_function_logs {
                let content_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(content_area);

                functions_timing::render_functions_table(frame, app, content_chunks[0]);
                timing_logs::render_function_logs_panel(
                    app.current_timing_logs.as_ref(),
                    app.selected_function_name().as_deref(),
                    content_chunks[1],
                    frame,
                    &mut app.function_logs_table_state,
                    app.functions_focus == FunctionsFocus::Logs,
                );

                if app.functions_focus == FunctionsFocus::Inspect {
                    if let Some(ref inspected_log) = app.inspected_function_log {
                        timing_inspect::render_inspect_popup(
                            inspected_log,
                            content_area,
                            frame,
                            app.timing_functions.total_elapsed_ns,
                        );
                    }
                }
            } else {
                functions_timing::render_functions_table(frame, app, content_area);
            }
        }
        FunctionsSubTab::Memory => {
            if app.show_function_logs {
                let content_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(content_area);

                functions_memory::render_functions_table(frame, app, content_chunks[0]);
                memory_logs::render_function_logs_panel(
                    app.current_alloc_logs.as_ref(),
                    app.selected_function_name().as_deref(),
                    content_chunks[1],
                    frame,
                    &mut app.function_logs_table_state,
                    app.functions_focus == FunctionsFocus::Logs,
                );

                if app.functions_focus == FunctionsFocus::Inspect {
                    if let Some(ref inspected_log) = app.inspected_function_log {
                        memory_inspect::render_inspect_popup(
                            inspected_log,
                            content_area,
                            frame,
                            app.memory_functions.total_elapsed_ns,
                        );
                    }
                }
            } else {
                functions_memory::render_functions_table(frame, app, content_area);
            }
        }
        FunctionsSubTab::Cpu => {
            functions_cpu::render_functions_table(frame, app, content_area);
        }
    }
}

fn sub_tab_label(name: &str, active: bool) -> Span<'static> {
    if active {
        Span::styled(
            format!(" {}*", name),
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!(" {} ", name), Style::default().fg(Color::Gray))
    }
}

/// Renders a sub-tab row (`[N] Label |Label |...`) and records each label's
/// screen region into `hit_areas` for mouse hit testing.
fn render_sub_tab_line(
    frame: &mut Frame,
    area: Rect,
    prefix: &'static str,
    labels: Vec<(Span<'static>, SubTabHit)>,
    hit_areas: &mut Vec<(Rect, SubTabHit)>,
) {
    let mut spans = vec![Span::styled(
        prefix,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];
    let mut x = area.x.saturating_add(prefix.len() as u16);
    let last = labels.len().saturating_sub(1);
    for (i, (span, hit)) in labels.into_iter().enumerate() {
        let width = span.width() as u16;
        let rect = Rect {
            x,
            y: area.y,
            width,
            height: 1,
        }
        .intersection(area);
        hit_areas.push((rect, hit));
        spans.push(span);
        x = x.saturating_add(width);
        if i != last {
            spans.push(Span::raw("|"));
            x = x.saturating_add(1);
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[hotpath::measure]
fn render_functions_subtabs(frame: &mut Frame, area: Rect, app: &mut App) {
    let sub_tab = app.functions_sub_tab;
    let labels = [
        FunctionsSubTab::Timing,
        FunctionsSubTab::Memory,
        FunctionsSubTab::Cpu,
    ]
    .into_iter()
    .map(|tab| {
        (
            sub_tab_label(tab.name(), tab == sub_tab),
            SubTabHit::Functions(tab),
        )
    })
    .collect();
    render_sub_tab_line(frame, area, " [1]", labels, &mut app.sub_tab_hit_areas);
}

#[hotpath::measure]
fn render_data_flow_subtabs(frame: &mut Frame, area: Rect, app: &mut App) {
    let sub_tab = app.data_flow_sub_tab;
    let labels = [
        DataFlowSubTab::Channels,
        DataFlowSubTab::Streams,
        DataFlowSubTab::Futures,
        DataFlowSubTab::RwLocks,
        DataFlowSubTab::Mutexes,
    ]
    .into_iter()
    .map(|tab| {
        (
            sub_tab_label(tab.name(), tab == sub_tab),
            SubTabHit::DataFlow(tab),
        )
    })
    .collect();
    render_sub_tab_line(frame, area, " [2]", labels, &mut app.sub_tab_hit_areas);
}

#[hotpath::measure]
fn render_io_subtabs(frame: &mut Frame, area: Rect, app: &mut App) {
    let sub_tab = app.io_sub_tab;
    let labels = [
        IoSubTab::Sql,
        IoSubTab::Http,
        IoSubTab::Server,
        IoSubTab::Bytes,
    ]
    .into_iter()
    .map(|tab| {
        (
            sub_tab_label(tab.name(), tab == sub_tab),
            SubTabHit::Io(tab),
        )
    })
    .collect();
    render_sub_tab_line(frame, area, " [3]", labels, &mut app.sub_tab_hit_areas);
}

#[hotpath::measure]
fn render_io_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let total = app.io_entries_len();

    if let Some(ref error_msg) = app.error_message {
        if total == 0 {
            let error_text = vec![
                Line::from(""),
                Line::from("Error").red().bold().centered(),
                Line::from(""),
                Line::from(error_msg.as_str()).red().centered(),
                Line::from(""),
                Line::from(format!(
                    "Make sure the metrics server is running on {}",
                    app.metrics_host
                ))
                .yellow()
                .centered(),
            ];

            let block = Block::bordered().border_set(border::THICK);
            frame.render_widget(Paragraph::new(error_text).block(block), area);
            return;
        }
    }

    if total == 0 {
        let empty_lines = match app.io_sub_tab {
            IoSubTab::Sql => vec![
                Line::from(""),
                Line::from("No SQL queries found").yellow().centered(),
                Line::from(""),
                Line::from("Add hotpath::sqlx_tracing_layer() to your tracing subscriber")
                    .centered(),
            ],
            IoSubTab::Http => vec![
                Line::from(""),
                Line::from("No HTTP requests found").yellow().centered(),
                Line::from(""),
                Line::from("Wrap your reqwest client with hotpath::http!(...)").centered(),
            ],
            IoSubTab::Server => vec![
                Line::from(""),
                Line::from("No served requests found").yellow().centered(),
                Line::from(""),
                Line::from("Wrap your axum router with hotpath::axum!(...)").centered(),
            ],
            IoSubTab::Bytes => vec![
                Line::from(""),
                Line::from("No I/O wrappers found").yellow().centered(),
                Line::from(""),
                Line::from(
                    "Use the io! macro to instrument Read/Write/AsyncRead/AsyncWrite values",
                )
                .centered(),
            ],
        };

        let block = Block::bordered().border_set(border::THICK);
        frame.render_widget(Paragraph::new(empty_lines).block(block), area);
        return;
    }

    let show_logs = match app.io_sub_tab {
        IoSubTab::Sql => app.show_sql_logs,
        IoSubTab::Http | IoSubTab::Server => app.show_http_logs,
        IoSubTab::Bytes => false,
    };
    let (table_area, logs_area) = if show_logs {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let selected_index = match app.io_sub_tab {
        IoSubTab::Sql => app.sql_table_state.selected().unwrap_or(0),
        IoSubTab::Http => app.http_table_state.selected().unwrap_or(0),
        IoSubTab::Server => app.server_table_state.selected().unwrap_or(0),
        IoSubTab::Bytes => app.io_bytes_table_state.selected().unwrap_or(0),
    };
    let position = selected_index + 1;

    match app.io_sub_tab {
        IoSubTab::Sql => data_flow::render_sql_panel(
            &app.sql.data,
            &app.sql.percentiles,
            app.sql.total_ns,
            app.sql.total_calls,
            table_area,
            frame,
            &mut app.sql_table_state,
            app.show_sql_logs,
            app.io_focus == IoFocus::List,
            position,
            total,
        ),
        IoSubTab::Http => data_flow::render_http_panel(
            &app.http.data,
            &app.http.percentiles,
            app.http.total_ns,
            app.http.total_calls,
            table_area,
            frame,
            &mut app.http_table_state,
            app.show_http_logs,
            app.io_focus == IoFocus::List,
            position,
            total,
        ),
        IoSubTab::Server => data_flow::render_server_panel(
            &app.server.data,
            &app.server.percentiles,
            app.server.total_ns,
            app.server.total_calls,
            app.server.total_alloc_bytes,
            table_area,
            frame,
            &mut app.server_table_state,
            app.show_http_logs,
            app.io_focus == IoFocus::List,
            position,
            total,
        ),
        IoSubTab::Bytes => data_flow::render_io_bytes_panel(
            &app.io_bytes.data,
            &app.io_bytes.percentiles,
            table_area,
            frame,
            &mut app.io_bytes_table_state,
            position,
            total,
        ),
    };

    if let Some(logs_area) = logs_area {
        let message = if app.paused {
            "(refresh paused)"
        } else if app.error_message.is_some() {
            "(cannot fetch new data)"
        } else {
            "(no data)"
        };

        match app.io_sub_tab {
            IoSubTab::Sql => {
                let selected = app
                    .sql_table_state
                    .selected()
                    .and_then(|i| app.sql.data.get(i));
                let label = selected
                    .map(|e| truncate_message(&e.query, 48))
                    .unwrap_or_else(|| "Unknown".to_string());
                let source = selected.and_then(|e| e.source.as_deref());

                if let Some(ref logs) = app.sql_logs {
                    io_logs::render_sql_logs_panel(
                        logs,
                        &label,
                        source,
                        logs_area,
                        frame,
                        &mut app.sql_logs_table_state,
                        app.io_focus == IoFocus::Logs,
                    );
                } else {
                    data_flow_logs::render_logs_placeholder(&label, message, logs_area, frame);
                }
            }
            IoSubTab::Http => {
                let selected = app
                    .http_table_state
                    .selected()
                    .and_then(|i| app.http.data.get(i));
                let label = selected
                    .map(|e| truncate_message(&e.endpoint, 48))
                    .unwrap_or_else(|| "Unknown".to_string());
                let source = selected.and_then(|e| e.source.as_deref());

                if let Some(ref logs) = app.http_logs {
                    io_logs::render_http_logs_panel(
                        logs,
                        &label,
                        source,
                        logs_area,
                        frame,
                        &mut app.http_logs_table_state,
                        app.io_focus == IoFocus::Logs,
                    );
                } else {
                    data_flow_logs::render_logs_placeholder(&label, message, logs_area, frame);
                }
            }
            IoSubTab::Server => {
                let label = app
                    .server_table_state
                    .selected()
                    .and_then(|i| app.server.data.get(i))
                    .map(|e| truncate_message(&e.route, 48))
                    .unwrap_or_else(|| "Unknown".to_string());

                if let Some(ref logs) = app.http_logs {
                    io_logs::render_http_logs_panel(
                        logs,
                        &label,
                        None,
                        logs_area,
                        frame,
                        &mut app.http_logs_table_state,
                        app.io_focus == IoFocus::Logs,
                    );
                } else {
                    data_flow_logs::render_logs_placeholder(&label, message, logs_area, frame);
                }
            }
            IoSubTab::Bytes => {}
        }
    }

    if app.io_focus == IoFocus::Inspect {
        match app.io_sub_tab {
            IoSubTab::Sql => {
                if let Some(ref inspected) = app.inspected_sql_log {
                    let selected = app
                        .sql_table_state
                        .selected()
                        .and_then(|i| app.sql.data.get(i));
                    let source = selected.and_then(|e| e.source.as_deref());
                    let route = selected.and_then(|e| e.route.as_deref());
                    io_inspect::render_sql_inspect_popup(inspected, source, route, area, frame);
                }
            }
            IoSubTab::Http => {
                if let Some(ref inspected) = app.inspected_http_log {
                    let selected = app
                        .http_table_state
                        .selected()
                        .and_then(|i| app.http.data.get(i));
                    let source = selected.and_then(|e| e.source.as_deref());
                    let route = selected.and_then(|e| e.route.as_deref());
                    io_inspect::render_http_inspect_popup(inspected, source, route, area, frame);
                }
            }
            IoSubTab::Server => {
                if let Some(ref inspected) = app.inspected_http_log {
                    let route = app
                        .server_table_state
                        .selected()
                        .and_then(|i| app.server.data.get(i))
                        .map(|e| e.route.as_str());
                    io_inspect::render_http_inspect_popup(inspected, None, route, area, frame);
                }
            }
            IoSubTab::Bytes => {}
        }
    }
}

#[hotpath::measure]
fn render_data_flow_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let total = app.data_flow_entries_len();

    if let Some(ref error_msg) = app.error_message {
        if total == 0 {
            let error_text = vec![
                Line::from(""),
                Line::from("Error").red().bold().centered(),
                Line::from(""),
                Line::from(error_msg.as_str()).red().centered(),
                Line::from(""),
                Line::from(format!(
                    "Make sure the metrics server is running on {}",
                    app.metrics_host
                ))
                .yellow()
                .centered(),
            ];

            let block = Block::bordered().border_set(border::THICK);
            frame.render_widget(Paragraph::new(error_text).block(block), area);
            return;
        }
    }

    if total == 0 {
        let empty_lines = match app.data_flow_sub_tab {
            DataFlowSubTab::Channels => vec![
                Line::from(""),
                Line::from("No channels found").yellow().centered(),
                Line::from(""),
                Line::from("Use the channel! macro to instrument channels").centered(),
            ],
            DataFlowSubTab::Streams => vec![
                Line::from(""),
                Line::from("No streams found").yellow().centered(),
                Line::from(""),
                Line::from("Use the stream! macro to instrument streams").centered(),
            ],
            DataFlowSubTab::Futures => vec![
                Line::from(""),
                Line::from("No futures found").yellow().centered(),
                Line::from(""),
                Line::from("Use the future! macro to instrument futures").centered(),
            ],
            DataFlowSubTab::RwLocks => vec![
                Line::from(""),
                Line::from("No RwLocks found").yellow().centered(),
                Line::from(""),
                Line::from("Use the rw_lock! macro to instrument RwLocks").centered(),
            ],
            DataFlowSubTab::Mutexes => vec![
                Line::from(""),
                Line::from("No Mutexes found").yellow().centered(),
                Line::from(""),
                Line::from("Use the mutex! macro to instrument Mutexes").centered(),
            ],
        };

        let block = Block::bordered().border_set(border::THICK);
        frame.render_widget(Paragraph::new(empty_lines).block(block), area);
        return;
    }

    let (table_area, logs_area) = if app.show_data_flow_logs {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let selected_index = app.data_flow_table_state().selected().unwrap_or(0);
    let position = selected_index + 1;

    let label = match app.data_flow_sub_tab {
        DataFlowSubTab::Channels => app
            .channels_table_state
            .selected()
            .and_then(|i| app.channels.data.get(i))
            .and_then(|e| {
                if e.label.is_empty() {
                    e.id.map(|id| id.to_string())
                } else {
                    Some(e.label.clone())
                }
            })
            .unwrap_or_else(|| "Unknown".to_string()),
        DataFlowSubTab::Streams => app
            .streams_table_state
            .selected()
            .and_then(|i| app.streams.data.get(i))
            .and_then(|e| {
                if e.label.is_empty() {
                    e.id.map(|id| id.to_string())
                } else {
                    Some(e.label.clone())
                }
            })
            .unwrap_or_else(|| "Unknown".to_string()),
        DataFlowSubTab::Futures => app
            .futures_table_state
            .selected()
            .and_then(|i| app.futures.data.get(i))
            .and_then(|e| {
                if e.label.is_empty() {
                    e.id.map(|id| id.to_string())
                } else {
                    Some(e.label.clone())
                }
            })
            .unwrap_or_else(|| "Unknown".to_string()),
        DataFlowSubTab::RwLocks => String::new(),
        DataFlowSubTab::Mutexes => String::new(),
    };

    match app.data_flow_sub_tab {
        DataFlowSubTab::Channels => data_flow::render_channels_panel(
            &app.channels.data,
            &app.channels.percentiles,
            table_area,
            frame,
            &mut app.channels_table_state,
            app.show_data_flow_logs,
            app.data_flow_focus,
            position,
            total,
        ),
        DataFlowSubTab::Streams => data_flow::render_streams_panel(
            &app.streams.data,
            table_area,
            frame,
            &mut app.streams_table_state,
            app.show_data_flow_logs,
            app.data_flow_focus,
            position,
            total,
        ),
        DataFlowSubTab::Futures => data_flow::render_futures_panel(
            &app.futures.data,
            table_area,
            frame,
            &mut app.futures_table_state,
            app.show_data_flow_logs,
            app.data_flow_focus,
            position,
            total,
        ),
        DataFlowSubTab::RwLocks => data_flow::render_rw_locks_panel(
            &app.rw_locks.data,
            &app.rw_locks.percentiles,
            table_area,
            frame,
            &mut app.rw_locks_table_state,
            position,
            total,
        ),
        DataFlowSubTab::Mutexes => data_flow::render_mutexes_panel(
            &app.mutexes.data,
            &app.mutexes.percentiles,
            table_area,
            frame,
            &mut app.mutexes_table_state,
            position,
            total,
        ),
    };

    if let Some(logs_area) = logs_area {
        if let Some(ref logs) = app.data_flow_logs {
            let has_missing_log = match logs {
                DataFlowLogs::Channel(l) => l.sent_logs.iter().any(|e| e.message.is_none()),
                DataFlowLogs::Stream(l) => l.logs.iter().any(|e| e.message.is_none()),
                DataFlowLogs::Future(_) => false,
            };
            data_flow_logs::render_logs_panel(
                logs,
                app.data_flow_sub_tab,
                &label,
                has_missing_log,
                logs_area,
                frame,
                &mut app.data_flow_logs_table_state,
                app.data_flow_focus == DataFlowFocus::Logs,
            );
        } else {
            let message = if app.paused {
                "(refresh paused)"
            } else if app.error_message.is_some() {
                "(cannot fetch new data)"
            } else {
                "(no data)"
            };
            data_flow_logs::render_logs_placeholder(&label, message, logs_area, frame);
        }
    }

    if app.data_flow_focus == DataFlowFocus::Inspect {
        if let Some(ref inspected) = app.inspected_data_flow_log {
            data_flow_inspect::render_inspect_popup(inspected, &label, area, frame);
        }
    }
}

#[hotpath::measure]
fn render_threads_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let thread_list = &app.threads.data;

    if let Some(ref error_msg) = app.error_message {
        if thread_list.is_empty() {
            let error_text = vec![
                Line::from(""),
                Line::from("Error").red().bold().centered(),
                Line::from(""),
                Line::from(error_msg.as_str()).red().centered(),
                Line::from(""),
                Line::from(format!(
                    "Make sure the metrics server is running on {}",
                    app.metrics_host
                ))
                .yellow()
                .centered(),
            ];

            let block = Block::bordered().border_set(border::THICK);
            frame.render_widget(Paragraph::new(error_text).block(block), area);
            return;
        }
    }

    if thread_list.is_empty() {
        let empty_text = vec![
            Line::from(""),
            Line::from("No thread statistics found").yellow().centered(),
            Line::from(""),
            Line::from("Make sure thread monitoring is enabled and the server is running")
                .centered(),
        ];

        let block = Block::bordered().border_set(border::THICK);
        frame.render_widget(Paragraph::new(empty_text).block(block), area);
        return;
    }

    let selected_index = app.threads_table_state.selected().unwrap_or(0);
    let thread_position = selected_index + 1;
    let total_threads = thread_list.len();

    threads::render_threads_panel(
        thread_list,
        area,
        frame,
        &mut app.threads_table_state,
        thread_position,
        total_threads,
        app.threads.rss_bytes.as_deref(),
        app.threads.rss_bytes_max.as_deref(),
        app.threads.total_alloc_bytes.as_deref(),
        app.threads.total_dealloc_bytes.as_deref(),
        app.threads.alloc_dealloc_diff.as_deref(),
    );
}

#[hotpath::measure]
fn render_debug_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let stats = &app.debug_stats;

    if let Some(ref error_msg) = app.error_message {
        if stats.is_empty() {
            let error_text = vec![
                Line::from(""),
                Line::from("Error").red().bold().centered(),
                Line::from(""),
                Line::from(error_msg.as_str()).red().centered(),
                Line::from(""),
                Line::from(format!(
                    "Make sure the metrics server is running on {}",
                    app.metrics_host
                ))
                .yellow()
                .centered(),
            ];

            let block = Block::bordered().border_set(border::THICK);
            frame.render_widget(Paragraph::new(error_text).block(block), area);
            return;
        }
    }

    if stats.is_empty() {
        let empty_text = vec![
            Line::from(""),
            Line::from("No debug logs found").yellow().centered(),
            Line::from(""),
            Line::from("Use hotpath::dbg!, hotpath::gauge!, or hotpath::val! macros.").centered(),
        ];

        let block = Block::bordered().border_set(border::THICK);
        frame.render_widget(Paragraph::new(empty_text).block(block), area);
        return;
    }

    let (table_area, logs_area) = if app.show_debug_logs {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let selected_index = app.debug_table_state.selected().unwrap_or(0);
    let debug_position = selected_index + 1;
    let total_debug = stats.len();

    debug::render_debug_panel(
        stats,
        table_area,
        frame,
        &mut app.debug_table_state,
        app.show_debug_logs,
        app.debug_focus,
        debug_position,
        total_debug,
    );

    if let Some(logs_area) = logs_area {
        let source_label = app
            .debug_table_state
            .selected()
            .and_then(|i| stats.get(i))
            .map(|stat| stat.source_display.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        if let Some(ref cached_logs) = app.debug_logs {
            debug_logs::render_debug_logs_panel(
                cached_logs,
                &source_label,
                logs_area,
                frame,
                &mut app.debug_logs_table_state,
                app.debug_focus == DebugFocus::Logs,
            );
        } else {
            let message = if app.paused {
                "(refresh paused)"
            } else if app.error_message.is_some() {
                "(cannot fetch new data)"
            } else {
                "(no data)"
            };
            debug_logs::render_debug_logs_placeholder(&source_label, message, logs_area, frame);
        }
    }

    if app.debug_focus == DebugFocus::Inspect {
        if let Some(ref inspected_log) = app.inspected_debug_log {
            debug_inspect::render_debug_inspect_popup(inspected_log, area, frame);
        }
    }
}

#[hotpath::measure]
fn render_runtime_view(frame: &mut Frame, app: &mut App, area: Rect) {
    let snap = app.tokio_runtime.as_ref();

    if let Some(ref error_msg) = app.error_message {
        if snap.is_none() {
            let error_text = vec![
                Line::from(""),
                Line::from("Error").red().bold().centered(),
                Line::from(""),
                Line::from(error_msg.as_str()).red().centered(),
            ];

            let block = Block::bordered().border_set(border::THICK);
            frame.render_widget(Paragraph::new(error_text).block(block), area);
            return;
        }
    }

    let Some(snap) = snap else {
        let empty_text = vec![
            Line::from(""),
            Line::from("No Tokio runtime metrics available")
                .yellow()
                .centered(),
            Line::from(""),
            Line::from("Use hotpath::tokio_runtime!() in your application").centered(),
        ];

        let block = Block::bordered().border_set(border::THICK);
        frame.render_widget(Paragraph::new(empty_text).block(block), area);
        return;
    };

    let selected_index = app.runtime_table_state.selected().unwrap_or(0);
    let worker_position = selected_index + 1;
    let total_workers = snap.workers.len();

    runtime::render_runtime_panel(
        snap,
        area,
        frame,
        &mut app.runtime_table_state,
        worker_position,
        total_workers,
    );
}

#[hotpath::measure]
fn render_tabs(frame: &mut Frame, area: Rect, app: &mut App) {
    let selected_tab = app.selected_tab;
    let create_tab_line = |tab: SelectedTab| {
        let name = if tab == selected_tab {
            format!(" {}*", tab.name())
        } else {
            format!(" {} ", tab.name())
        };
        Line::from(vec![
            Span::styled(
                format!("[{}]", tab.number()),
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(name, Style::default().fg(Color::Gray)),
        ])
    };

    let all_tabs = [
        SelectedTab::Functions,
        SelectedTab::DataFlow,
        SelectedTab::Io,
        SelectedTab::Threads,
        SelectedTab::Debug,
        SelectedTab::Runtime,
    ];
    let titles: Vec<Line> = all_tabs.iter().map(|tab| create_tab_line(*tab)).collect();

    // Mirror the `Tabs` widget layout: 1-char padding on each side of every
    // title, with the divider between tabs. The padded region is the hit area.
    let mut x = area.x;
    for (i, (tab, title)) in all_tabs.iter().zip(&titles).enumerate() {
        let width = (title.width() as u16).saturating_add(2);
        let rect = Rect {
            x,
            y: area.y,
            width,
            height: 1,
        }
        .intersection(area);
        app.tab_hit_areas.push((rect, *tab));
        x = x.saturating_add(width);
        if i != all_tabs.len() - 1 {
            x = x.saturating_add(" | ".len() as u16);
        }
    }

    let selected_index = (selected_tab.number() - 1) as usize;

    let tabs = Tabs::new(titles)
        .select(selected_index)
        .divider(" | ")
        .style(Style::default())
        .highlight_style(
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        );

    frame.render_widget(tabs, area);
}
