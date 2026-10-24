use crate::cmd::console::app::App;
use crate::cmd::console::constants::SAMPLY_LOAD_DISABLED;
use crate::cmd::console::views::common_styles;
use hotpath::json::CpuSnapshotStatus;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Cell, Paragraph, Row, Table},
    Frame,
};

#[hotpath::measure]
pub(crate) fn render_functions_table(frame: &mut Frame, app: &mut App, area: Rect) {
    if let Some(reason) = app.cpu_unavailable_reason.as_deref() {
        render_unavailable(frame, area, reason);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    render_status_panel(frame, app, chunks[0]);
    render_cpu_table(frame, app, chunks[1]);
}

fn render_unavailable(frame: &mut Frame, area: Rect, reason: &str) {
    let block = Block::bordered()
        .border_set(border::THICK)
        .title(Span::styled(
            " CPU samples ",
            common_styles::TITLE_STYLE_YELLOW,
        ));

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            reason,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center);

    frame.render_widget(paragraph, area);
}

fn render_status_panel(frame: &mut Frame, app: &App, area: Rect) {
    let envelope = app.cpu_envelope.as_ref();
    let status = envelope
        .map(|e| e.status)
        .unwrap_or(CpuSnapshotStatus::Idle);

    let status_text = match status {
        CpuSnapshotStatus::Idle => Span::styled(
            "Idle",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        CpuSnapshotStatus::Capturing => Span::styled(
            "Capturing...",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        CpuSnapshotStatus::Ready => Span::styled(
            "Ready",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        CpuSnapshotStatus::Error => Span::styled(
            "Error",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
    };

    let mut detail_spans: Vec<Span> = Vec::new();
    detail_spans.push(Span::raw(" status: "));
    detail_spans.push(status_text);

    if let Some(env) = envelope {
        if let Some(session_id) = &env.current_session_id {
            detail_spans.push(Span::raw("  |  session: "));
            detail_spans.push(Span::styled(
                session_id.clone(),
                Style::default().fg(Color::Cyan),
            ));
        }
        if let Some(dur) = env.capture_duration_ms {
            detail_spans.push(Span::raw("  |  capture took "));
            detail_spans.push(Span::styled(
                format!("{} ms", dur),
                Style::default().fg(Color::Cyan),
            ));
        }
        if let Some(err) = &env.error {
            detail_spans.push(Span::raw("  |  "));
            detail_spans.push(Span::styled(err.clone(), Style::default().fg(Color::Red)));
        }
        if env.report.is_some() && env.status == CpuSnapshotStatus::Ready && !*SAMPLY_LOAD_DISABLED
        {
            detail_spans.push(Span::raw("  |  "));
            detail_spans.push(Span::styled(
                "press 'f' to open in samply",
                Style::default().fg(Color::Magenta),
            ));
        }
    }

    let line = Line::from(detail_spans);
    let title = Span::styled(" CPU snapshot ", common_styles::TITLE_STYLE_YELLOW);
    let block = Block::bordered().title(title).border_set(border::THICK);
    let widget = Paragraph::new(line).block(block);
    frame.render_widget(widget, area);
}

fn render_cpu_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let envelope = app.cpu_envelope.as_ref();
    let status = envelope
        .map(|e| e.status)
        .unwrap_or(CpuSnapshotStatus::Idle);

    let title = " CPU samples ".to_string();

    if status != CpuSnapshotStatus::Ready || envelope.and_then(|e| e.report.as_ref()).is_none() {
        let hint = match status {
            CpuSnapshotStatus::Idle => "Press 'c' to capture a CPU snapshot",
            CpuSnapshotStatus::Capturing => "Capturing CPU profile, please wait...",
            CpuSnapshotStatus::Error => "Snapshot failed - press 'c' to retry",
            CpuSnapshotStatus::Ready => "No CPU samples in last snapshot",
        };
        let block = Block::bordered()
            .title(Span::styled(title, common_styles::TITLE_STYLE_YELLOW))
            .border_set(border::THICK);
        let para = Paragraph::new(Line::from(Span::raw(hint))).block(block);
        frame.render_widget(para, area);
        return;
    }

    let report = envelope
        .and_then(|e| e.report.as_ref())
        .expect("checked above");

    let header_cells = ["Function", "Samples", "% Total"]
        .iter()
        .map(|h| Cell::from(*h).style(common_styles::HEADER_STYLE_CYAN))
        .collect::<Vec<_>>();
    let header = Row::new(header_cells).height(1);

    let total_rows = report.data.len();
    let position = app.cpu_table_state.selected().map(|s| s + 1).unwrap_or(0);

    let rows = report.data.iter().map(|entry| {
        let short_name = hotpath::shorten_function_name(&entry.name);
        Row::new(vec![
            Cell::from(short_name),
            Cell::from(hotpath::format_count_display(entry.samples)),
            Cell::from(entry.percent.clone()),
        ])
    });

    let table = Table::new(
        rows,
        vec![
            Constraint::Percentage(60),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(
        Block::bordered()
            .title(format!(" [{}/{}] ", position, total_rows))
            .title(Span::styled(title, common_styles::TITLE_STYLE_YELLOW))
            .border_set(border::THICK),
    )
    .row_highlight_style(common_styles::SELECTED_ROW_STYLE)
    .highlight_symbol(">> ");

    frame.render_stateful_widget(table, area, &mut app.cpu_table_state);
}
