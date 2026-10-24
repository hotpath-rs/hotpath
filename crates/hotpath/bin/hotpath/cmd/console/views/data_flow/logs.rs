use crate::cmd::console::app::{DataFlowLogs, DataFlowSubTab};
use crate::cmd::console::views::common_styles;
use crate::cmd::console::widgets::formatters::truncate_message;
use hotpath::{format_bytes, format_duration};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
    Frame,
};

pub(crate) fn render_logs_placeholder(label: &str, message: &str, area: Rect, frame: &mut Frame) {
    let block = Block::bordered()
        .title(format!(" {} ", label))
        .border_set(border::THICK);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let message_width = message.len() as u16;
    let x = inner_area.x + (inner_area.width.saturating_sub(message_width)) / 2;
    let y = inner_area.y + inner_area.height / 2;

    if x < inner_area.x + inner_area.width && y < inner_area.y + inner_area.height {
        frame
            .buffer_mut()
            .set_string(x, y, message, common_styles::PLACEHOLDER_STYLE);
    }
}

fn state_style(state: &str) -> Style {
    match state {
        "Ready" => Style::default().fg(Color::Green),
        "Cancelled" => Style::default().fg(Color::Red),
        "Suspended" => Style::default().fg(Color::Yellow),
        "Running" => Style::default().fg(Color::Blue),
        "Pending" => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_logs_panel(
    logs: &DataFlowLogs,
    sub_tab: DataFlowSubTab,
    label: &str,
    has_missing_log: bool,
    area: Rect,
    frame: &mut Frame,
    table_state: &mut TableState,
    is_focused: bool,
) {
    let title_style = Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD);

    let title = if has_missing_log {
        Line::from(vec![
            Span::styled(format!(" {} ", label), title_style),
            Span::styled(
                "(missing \"log = true\") ",
                Style::default().fg(Color::DarkGray),
            ),
        ])
    } else {
        Line::from(Span::styled(format!(" {} ", label), title_style))
    };

    let border_set = if is_focused {
        border::THICK
    } else {
        border::PLAIN
    };

    let block = Block::bordered()
        .title(title)
        .border_set(border_set)
        .border_style(if is_focused {
            Style::default()
        } else {
            common_styles::UNFOCUSED_BORDER_STYLE
        });

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let available_width = inner_area.width.saturating_sub(2);
    let msg_width = (available_width.saturating_sub(30) as usize).max(20);

    match (logs, sub_tab) {
        (DataFlowLogs::Channel(channel_logs), DataFlowSubTab::Channels) => {
            let header = Row::new(vec!["Index", "Message", "Delay", "Ago"])
                .style(common_styles::HEADER_STYLE_CYAN)
                .height(1);

            let rows: Vec<Row> = channel_logs
                .sent_logs
                .iter()
                .map(|entry| {
                    let msg = entry.message.as_deref().unwrap_or("");
                    let truncated_msg = truncate_message(msg, msg_width);
                    let delay_str = entry.delay.as_deref().unwrap_or("queued");

                    Row::new(vec![
                        entry.index.to_string(),
                        truncated_msg,
                        delay_str.to_string(),
                        entry.ago.clone(),
                    ])
                })
                .collect();

            let widths = [
                Constraint::Length(6),  // Index
                Constraint::Min(20),    // Message
                Constraint::Length(12), // Delay
                Constraint::Length(13), // Ago
            ];

            let table = Table::new(rows, widths)
                .header(header)
                .row_highlight_style(common_styles::SELECTED_ROW_STYLE)
                .highlight_symbol(">> ")
                .highlight_spacing(HighlightSpacing::Always);

            frame.render_stateful_widget(table, inner_area, table_state);
        }
        (DataFlowLogs::Stream(stream_logs), DataFlowSubTab::Streams) => {
            let header = Row::new(vec!["Index", "Message", "Ago"])
                .style(common_styles::HEADER_STYLE_CYAN)
                .height(1);

            let rows: Vec<Row> = stream_logs
                .logs
                .iter()
                .map(|entry| {
                    let msg = entry.message.as_deref().unwrap_or("");
                    let truncated_msg = truncate_message(msg, msg_width);

                    Row::new(vec![
                        entry.index.to_string(),
                        truncated_msg,
                        entry.ago.clone(),
                    ])
                })
                .collect();

            let widths = [
                Constraint::Length(6),  // Index
                Constraint::Min(20),    // Message
                Constraint::Length(13), // Ago
            ];

            let table = Table::new(rows, widths)
                .header(header)
                .row_highlight_style(common_styles::SELECTED_ROW_STYLE)
                .highlight_symbol(">> ")
                .highlight_spacing(HighlightSpacing::Always);

            frame.render_stateful_widget(table, inner_area, table_state);
        }
        (DataFlowLogs::Future(future_logs), DataFlowSubTab::Futures) => {
            let result_width = (available_width.saturating_sub(44) as usize).max(10);
            let total_alloc = future_logs
                .total_poll_alloc_bytes
                .map(format_bytes)
                .unwrap_or_else(|| "-".to_string());
            let no_sampled_polls = future_logs.total_polls > 0
                && future_logs.calls.iter().all(|c| c.sampled_polls == 0);
            let total_time = if no_sampled_polls {
                "N/A".to_string()
            } else {
                format_duration(future_logs.total_poll_duration_ns)
            };
            let summary = format!(
                "polls: {} | time: {} | alloc: {}",
                future_logs.total_polls, total_time, total_alloc,
            );

            let [summary_area, table_area] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner_area);
            frame.render_widget(Paragraph::new(Line::raw(summary)), summary_area);

            let header = Row::new(vec!["State", "Polls", "Time", "Alloc", "Result"])
                .style(common_styles::HEADER_STYLE_CYAN)
                .height(1);

            let rows: Vec<Row> = future_logs
                .calls
                .iter()
                .map(|call| {
                    let total_poll = if call.poll_count == 0 {
                        "-".to_string()
                    } else if call.sampled_polls == 0 {
                        "N/A".to_string()
                    } else {
                        format_duration(call.total_poll_duration_ns)
                    };
                    let alloc = call
                        .total_poll_alloc_bytes
                        .map(format_bytes)
                        .unwrap_or_else(|| "-".to_string());
                    let result = call.result.as_deref().unwrap_or("-");
                    let result_text = truncate_message(result, result_width);

                    Row::new(vec![
                        Cell::from(call.state.clone()).style(state_style(&call.state)),
                        Cell::from(hotpath::format_count_display(call.poll_count)),
                        Cell::from(total_poll),
                        Cell::from(alloc),
                        Cell::from(result_text),
                    ])
                })
                .collect();

            let widths = vec![
                Constraint::Length(9),  // State
                Constraint::Length(6),  // Polls
                Constraint::Length(11), // Time
                Constraint::Length(12), // Alloc
                Constraint::Min(10),    // Result
            ];

            let table = Table::new(rows, widths)
                .header(header)
                .row_highlight_style(common_styles::SELECTED_ROW_STYLE)
                .highlight_symbol(">> ")
                .highlight_spacing(HighlightSpacing::Always);

            frame.render_stateful_widget(table, table_area, table_state);
        }
        _ => {}
    }
}
