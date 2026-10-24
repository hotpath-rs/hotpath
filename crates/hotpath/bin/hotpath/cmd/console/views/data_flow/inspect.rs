use crate::cmd::console::app::InspectedDataFlowLog;
use hotpath::{format_bytes, format_duration};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    symbols::border,
    text::Line,
    widgets::{Block, Clear, Paragraph, Wrap},
    Frame,
};

fn wrap_text(text: &str, max_width: usize) -> Vec<Line<'static>> {
    // A zero width would make the split loop below yield empty slices forever.
    let max_width = max_width.max(1);
    text.lines()
        .flat_map(|line| {
            if line.len() <= max_width {
                vec![Line::from(line.to_string())]
            } else {
                let mut wrapped = Vec::new();
                let mut remaining = line;
                while !remaining.is_empty() {
                    let split_at = remaining
                        .char_indices()
                        .nth(max_width)
                        .map(|(i, _)| i)
                        .unwrap_or(remaining.len());
                    wrapped.push(Line::from(remaining[..split_at].to_string()));
                    remaining = &remaining[split_at..];
                }
                wrapped
            }
        })
        .collect()
}

fn format_opt_bytes(bytes: Option<u64>) -> String {
    bytes.map(format_bytes).unwrap_or_else(|| "-".to_string())
}

fn format_opt_count(count: Option<u64>) -> String {
    count
        .map(hotpath::format_count_display)
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn render_inspect_popup(
    inspected: &InspectedDataFlowLog,
    item_label: &str,
    area: Rect,
    frame: &mut Frame,
) {
    let popup_width = (area.width as f32 * 0.8) as u16;
    let popup_height = (area.height as f32 * 0.8) as u16;
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(popup_height)) / 2;

    let popup_area = Rect {
        x: area.x + x,
        y: area.y + y,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    if let InspectedDataFlowLog::FutureCall(call) = inspected {
        let block = Block::bordered()
            .title(format!(" {} ", item_label))
            .border_set(border::DOUBLE);

        let inner_area = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let (avg_poll, max_poll, total_poll) = if call.poll_count == 0 {
            ("-".to_string(), "-".to_string(), "-".to_string())
        } else if call.sampled_polls == 0 {
            ("N/A".to_string(), "N/A".to_string(), "N/A".to_string())
        } else {
            (
                format_duration(
                    call.total_poll_duration_ns
                        .checked_div(call.poll_count)
                        .unwrap_or(0),
                ),
                format_duration(call.max_poll_duration_ns),
                format_duration(call.total_poll_duration_ns),
            )
        };

        let details_text = format!(
            "State: {} | Polls: {}\nTiming: avg {} | max {} | total {}\nAlloc bytes/count: {} / {}",
            call.state,
            hotpath::format_count_display(call.poll_count),
            avg_poll,
            max_poll,
            total_poll,
            format_opt_bytes(call.total_poll_alloc_bytes),
            format_opt_count(call.total_poll_alloc_count),
        );

        let [details_area, _, result_area] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(inner_area);

        let details = Paragraph::new(details_text).wrap(Wrap { trim: false });
        frame.render_widget(details, details_area);

        let result_text = call
            .result
            .as_deref()
            .unwrap_or("(no result available)")
            .to_string();
        let max_width = result_area.width.saturating_sub(2) as usize;
        let result_lines = wrap_text(&result_text, max_width);
        let result = Paragraph::new(result_lines).wrap(Wrap { trim: false });
        frame.render_widget(result, result_area);

        return;
    }

    let message = match inspected {
        InspectedDataFlowLog::ChannelSent(entry) => entry
            .message
            .as_deref()
            .unwrap_or("(missing \"log = true\")")
            .to_string(),
        InspectedDataFlowLog::Stream(entry) => entry
            .message
            .as_deref()
            .unwrap_or("(missing \"log = true\")")
            .to_string(),
        InspectedDataFlowLog::FutureCall(_) => unreachable!(),
    };

    let block = Block::bordered()
        .title(format!(" {} ", item_label))
        .border_set(border::DOUBLE);

    let inner_area = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let max_width = inner_area.width.saturating_sub(2) as usize;
    let text_lines = wrap_text(&message, max_width);

    let paragraph = Paragraph::new(text_lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner_area);
}
