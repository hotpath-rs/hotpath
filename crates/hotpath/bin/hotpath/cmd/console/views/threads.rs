use crate::cmd::console::views::common_styles;
use crate::cmd::console::widgets::formatters::truncate_right;
use hotpath::json::JsonThreadEntry;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
    Frame,
};

fn status_style(status: &str) -> Style {
    let color = match status.trim() {
        "Running" => Color::Green,
        "Sleeping" => Color::Reset,
        "Blocked" => Color::Red,
        "Stopped" => Color::Yellow,
        "Zombie" => Color::Magenta,
        "Dead" => Color::DarkGray,
        "Halted" => Color::Red,
        "Wakekill" => Color::Yellow,
        "Waking" => Color::Cyan,
        "Parked" => Color::DarkGray,
        "Idle" => Color::DarkGray,
        _ => Color::DarkGray,
    };
    Style::default().fg(color)
}

#[hotpath::measure]
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_threads_panel(
    threads: &[JsonThreadEntry],
    area: Rect,
    frame: &mut Frame,
    table_state: &mut TableState,
    thread_position: usize,
    total_threads: usize,
    rss_bytes: Option<&str>,
    rss_bytes_max: Option<&str>,
    total_alloc_bytes: Option<&str>,
    total_dealloc_bytes: Option<&str>,
    alloc_dealloc_diff: Option<&str>,
) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let info_area = chunks[0];
    let table_area = chunks[1];

    let alloc_enabled = threads.iter().any(|t| t.alloc_bytes.is_some());

    let rss_str = rss_bytes.unwrap_or("-");
    let rss_max_str = rss_bytes_max.unwrap_or("-");

    let mut spans: Vec<Span> = Vec::new();

    if let Some(alloc) = total_alloc_bytes {
        spans.push(Span::raw(" Alloc: "));
        spans.push(Span::styled(
            alloc,
            ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
        ));
        spans.push(Span::raw("  Dealloc: "));
        spans.push(Span::styled(
            total_dealloc_bytes.unwrap_or("-"),
            ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
        ));
        spans.push(Span::raw("  Diff: "));
        spans.push(Span::styled(
            alloc_dealloc_diff.unwrap_or("-"),
            ratatui::style::Style::default().fg(ratatui::style::Color::Green),
        ));
    } else if !alloc_enabled {
        spans.push(Span::styled(
            " Enable 'hotpath-alloc' to track memory usage",
            ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray),
        ));
    }

    spans.push(Span::raw("  RSS: "));
    spans.push(Span::styled(
        rss_str,
        ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
    ));
    spans.push(Span::raw("  Max RSS: "));
    spans.push(Span::styled(
        rss_max_str,
        ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
    ));

    let info_line = Line::from(spans);
    let info_paragraph = Paragraph::new(info_line);
    frame.render_widget(info_paragraph, info_area);

    let available_width = table_area.width.saturating_sub(10);
    let thread_width = ((available_width as f32 * 0.16) as usize).max(10);

    let header = Row::new(vec![
        Cell::from("Thread"),
        Cell::from("TID"),
        Cell::from("Status"),
        Cell::from("CPU %"),
        Cell::from("Max %"),
        Cell::from("Avg %"),
        Cell::from("Alloc"),
        Cell::from("Dealloc"),
        Cell::from("Diff"),
    ])
    .style(common_styles::HEADER_STYLE_CYAN)
    .height(1);

    let rows: Vec<Row> = threads
        .iter()
        .map(|thread| {
            let cpu_percent_str = thread.cpu_percent.as_deref().unwrap_or("-");
            let cpu_percent_max_str = thread.cpu_percent_max.as_deref().unwrap_or("-");

            let (alloc_str, dealloc_str, diff_str) = if alloc_enabled {
                (
                    thread.alloc_bytes.as_deref().unwrap_or("-"),
                    thread.dealloc_bytes.as_deref().unwrap_or("-"),
                    thread.mem_diff.as_deref().unwrap_or("-"),
                )
            } else {
                ("N/A", "N/A", "N/A")
            };

            let status_str = format!("{:<8} ({})", thread.status, thread.status_code);

            let cpu_percent_avg_str = thread.cpu_percent_avg.as_deref().unwrap_or("-");

            Row::new(vec![
                Cell::from(truncate_right(&thread.name, thread_width)),
                Cell::from(thread.os_tid.to_string()),
                Cell::from(status_str).style(status_style(&thread.status)),
                Cell::from(cpu_percent_str),
                Cell::from(cpu_percent_max_str),
                Cell::from(cpu_percent_avg_str),
                Cell::from(alloc_str),
                Cell::from(dealloc_str),
                Cell::from(diff_str),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(16), // Thread name
        Constraint::Percentage(7),  // TID
        Constraint::Percentage(17), // Status
        Constraint::Percentage(7),  // CPU %
        Constraint::Percentage(7),  // Max %
        Constraint::Percentage(7),  // Avg %
        Constraint::Percentage(12), // Alloc
        Constraint::Percentage(12), // Dealloc
        Constraint::Percentage(15), // Diff
    ];

    let title = " Threads - CPU usage and memory metrics. ";
    let table_block = Block::bordered()
        .title(format!(" [{}/{}] ", thread_position, total_threads))
        .title(Span::styled(title, common_styles::TITLE_STYLE_YELLOW))
        .border_set(border::THICK);

    let table = Table::new(rows, widths)
        .header(header)
        .block(table_block)
        .column_spacing(1)
        .row_highlight_style(common_styles::SELECTED_ROW_STYLE)
        .highlight_symbol(">> ")
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, table_area, table_state);
}
