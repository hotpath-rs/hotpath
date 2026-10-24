use super::common_styles;
use hotpath::json::{JsonRuntimeSnapshot, JsonRuntimeWorker};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
    Frame,
};

#[hotpath::measure]
pub(crate) fn render_runtime_panel(
    snapshot: &JsonRuntimeSnapshot,
    area: Rect,
    frame: &mut Frame,
    table_state: &mut TableState,
    worker_position: usize,
    total_workers: usize,
) {
    let has_unstable = snapshot
        .workers
        .first()
        .is_some_and(|w| w.poll_count.is_some());

    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let info_area = chunks[0];
    let table_area = chunks[1];

    render_summary_line(snapshot, has_unstable, info_area, frame);

    let header_cells = if has_unstable {
        vec![
            Cell::from("Worker"),
            Cell::from("Park"),
            Cell::from("Busy (ms)"),
            Cell::from("Polls"),
            Cell::from("Steals"),
            Cell::from("Steal Ops"),
            Cell::from("Overflow"),
            Cell::from("Local Q"),
            Cell::from("Mean Poll (μs)"),
        ]
    } else {
        vec![
            Cell::from("Worker"),
            Cell::from("Park"),
            Cell::from("Busy (ms)"),
        ]
    };

    let header = Row::new(header_cells)
        .style(common_styles::HEADER_STYLE_CYAN)
        .height(1);

    let rows: Vec<Row> = snapshot
        .workers
        .iter()
        .map(|w| worker_row(w, has_unstable))
        .collect();

    let widths = if has_unstable {
        vec![
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(12),
            Constraint::Percentage(10),
            Constraint::Percentage(10),
            Constraint::Percentage(12),
            Constraint::Percentage(12),
            Constraint::Percentage(10),
            Constraint::Percentage(14),
        ]
    } else {
        vec![
            Constraint::Percentage(30),
            Constraint::Percentage(35),
            Constraint::Percentage(35),
        ]
    };

    let title = " Tokio Runtime - per-worker metrics. ";
    let mut table_block = Block::bordered()
        .title(format!(" [{}/{}] ", worker_position, total_workers))
        .title(Span::styled(title, common_styles::TITLE_STYLE_YELLOW))
        .border_set(border::THICK);

    if !has_unstable {
        table_block = table_block.title_bottom(Line::from(Span::styled(
            " Use RUSTFLAGS=\"--cfg tokio_unstable\" for more runtime metrics. ",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let table = Table::new(rows, widths)
        .header(header)
        .block(table_block)
        .column_spacing(1)
        .row_highlight_style(common_styles::SELECTED_ROW_STYLE)
        .highlight_symbol(">> ")
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, table_area, table_state);
}

fn render_summary_line(
    snapshot: &JsonRuntimeSnapshot,
    has_unstable: bool,
    area: Rect,
    frame: &mut Frame,
) {
    let yellow = Style::default().fg(Color::Yellow);
    let cyan = Style::default().fg(Color::Cyan);

    let mut spans = vec![
        Span::raw(" Workers: "),
        Span::styled(
            hotpath::format_count_display(snapshot.num_workers as u64),
            yellow,
        ),
        Span::raw("  Tasks: "),
        Span::styled(
            hotpath::format_count_display(snapshot.num_alive_tasks as u64),
            cyan,
        ),
        Span::raw("  Global Q: "),
        Span::styled(
            hotpath::format_count_display(snapshot.global_queue_depth as u64),
            cyan,
        ),
    ];

    if has_unstable {
        if let Some(spawned) = snapshot.spawned_tasks_count {
            spans.push(Span::raw("  Spawned: "));
            spans.push(Span::styled(hotpath::format_count_display(spawned), cyan));
        }
        if let Some(remote) = snapshot.remote_schedule_count {
            spans.push(Span::raw("  Remote Sched: "));
            spans.push(Span::styled(hotpath::format_count_display(remote), cyan));
        }
        if let Some(blocking) = snapshot.num_blocking_threads {
            let idle = snapshot.num_idle_blocking_threads.unwrap_or(0);
            spans.push(Span::raw("  Blocking: "));
            spans.push(Span::styled(
                format!(
                    "{}/{}",
                    hotpath::format_count_display(idle as u64),
                    hotpath::format_count_display(blocking as u64)
                ),
                cyan,
            ));
        }
        if let Some(io_reg) = snapshot.io_driver_fd_registered_count {
            let io_dereg = snapshot.io_driver_fd_deregistered_count.unwrap_or(0);
            spans.push(Span::raw("  FDs: "));
            spans.push(Span::styled(
                hotpath::format_count_display(io_reg.saturating_sub(io_dereg)),
                cyan,
            ));
        }
        if let Some(ready) = snapshot.io_driver_ready_count {
            spans.push(Span::raw("  IO Ready: "));
            spans.push(Span::styled(hotpath::format_count_display(ready), cyan));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn worker_row(w: &JsonRuntimeWorker, has_unstable: bool) -> Row<'static> {
    let base = vec![
        Cell::from(format!("worker-{}", w.index)),
        Cell::from(hotpath::format_count_display(w.park_count)),
        Cell::from(hotpath::format_count_display(w.busy_duration_ms)),
    ];

    if has_unstable {
        let mut cells = base;
        cells.push(Cell::from(opt_u64(w.poll_count)));
        cells.push(Cell::from(opt_u64(w.steal_count)));
        cells.push(Cell::from(opt_u64(w.steal_operations)));
        cells.push(Cell::from(opt_u64(w.overflow_count)));
        cells.push(Cell::from(opt_usize(w.local_queue_depth)));
        cells.push(Cell::from(opt_u64(w.mean_poll_time_us)));
        Row::new(cells)
    } else {
        Row::new(base)
    }
}

fn opt_u64(v: Option<u64>) -> String {
    v.map(hotpath::format_count_display)
        .unwrap_or_else(|| "-".to_string())
}

fn opt_usize(v: Option<usize>) -> String {
    v.map(|n| hotpath::format_count_display(n as u64))
        .unwrap_or_else(|| "-".to_string())
}
