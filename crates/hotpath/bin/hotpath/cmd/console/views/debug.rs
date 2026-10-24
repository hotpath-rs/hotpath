pub(crate) mod inspect;
pub(crate) mod logs;

use crate::cmd::console::app::DebugFocus;
use crate::cmd::console::views::common_styles;
use crate::cmd::console::widgets::formatters::truncate_left;
use hotpath::json::{DebugEntryType, JsonDebugEntry};
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Style},
    symbols::border,
    text::Span,
    widgets::{Block, Cell, HighlightSpacing, Row, Table, TableState},
    Frame,
};

fn type_color(entry_type: DebugEntryType) -> Color {
    match entry_type {
        DebugEntryType::Dbg => Color::Cyan,
        DebugEntryType::Val => Color::Magenta,
        DebugEntryType::Gauge => Color::Yellow,
    }
}

#[hotpath::measure]
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_debug_panel(
    stats: &[JsonDebugEntry],
    area: Rect,
    frame: &mut Frame,
    table_state: &mut TableState,
    show_logs: bool,
    focus: DebugFocus,
    position: usize,
    total: usize,
) {
    let available_width = area.width.saturating_sub(10);
    let source_width = ((available_width as f32 * 0.22) as usize).max(12);
    let label_width = ((available_width as f32 * 0.22) as usize).max(12);
    let value_width = ((available_width as f32 * 0.30) as usize).max(15);

    let header = Row::new(vec![
        Cell::from("Type"),
        Cell::from("Source"),
        Cell::from("Key/Expr"),
        Cell::from("Last"),
        Cell::from("Count"),
    ])
    .style(common_styles::HEADER_STYLE_CYAN)
    .height(1);

    let rows: Vec<Row> = stats
        .iter()
        .map(|stat| {
            let last_value = stat.last_value.as_deref().unwrap_or("-");
            let entry_type = stat.entry_type.as_str();
            let color = type_color(stat.entry_type);
            let key_expr = match stat.entry_type {
                DebugEntryType::Dbg => stat.expression.clone(),
                DebugEntryType::Val | DebugEntryType::Gauge => format!("\"{}\"", stat.expression),
            };
            Row::new(vec![
                Cell::from(entry_type).style(Style::default().fg(color)),
                Cell::from(truncate_left(&stat.source_display, source_width)),
                Cell::from(truncate_left(&key_expr, label_width)),
                Cell::from(truncate_left(last_value, value_width)),
                Cell::from(hotpath::format_count_display(stat.log_count)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(7),
        Constraint::Percentage(22),
        Constraint::Percentage(22),
        Constraint::Percentage(30),
        Constraint::Percentage(10),
    ];

    let title = " Debug - Values, gauges, and expressions. ";
    let table_block = if show_logs {
        let border_set = if focus == DebugFocus::Debug {
            border::THICK
        } else {
            border::PLAIN
        };
        Block::bordered()
            .title(format!(" [{}/{}] ", position, total))
            .title(Span::styled(title, common_styles::TITLE_STYLE_YELLOW))
            .border_set(border_set)
            .border_style(if focus == DebugFocus::Debug {
                Style::default()
            } else {
                common_styles::UNFOCUSED_BORDER_STYLE
            })
    } else {
        Block::bordered()
            .title(format!(" [{}/{}] ", position, total))
            .title(Span::styled(title, common_styles::TITLE_STYLE_YELLOW))
            .border_set(border::THICK)
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(table_block)
        .column_spacing(1)
        .row_highlight_style(common_styles::SELECTED_ROW_STYLE)
        .highlight_symbol(">> ")
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, area, table_state);
}
