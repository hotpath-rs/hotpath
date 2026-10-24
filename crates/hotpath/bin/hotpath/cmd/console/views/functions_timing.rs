pub(crate) mod inspect;
pub(crate) mod logs;

use super::super::app::{App, FunctionsFocus};
use super::common_styles;
use ratatui::{
    layout::{Constraint, Rect},
    style::Style,
    symbols::border,
    text::Span,
    widgets::{Block, Cell, Row, Table},
    Frame,
};

#[hotpath::measure]
pub(crate) fn render_functions_table(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(
        " {} - {} ",
        app.timing_functions.caller_name, app.timing_functions.description
    );

    let header_cells = vec![
        "Function".to_string(),
        "Calls".to_string(),
        "Avg".to_string(),
    ]
    .into_iter()
    .chain(
        app.timing_functions
            .percentiles
            .iter()
            .map(|p| hotpath::format_percentile_header(*p))
            .collect::<Vec<_>>(),
    )
    .chain(vec!["Total".to_string(), "% Total".to_string()])
    .map(|h| Cell::from(h).style(common_styles::HEADER_STYLE_CYAN))
    .collect::<Vec<_>>();

    let header = Row::new(header_cells).height(1);

    let entries = app.get_timing_measurements();
    let total_functions = entries.len();
    let function_position = app
        .timing_table_state
        .selected()
        .map(|s| s + 1)
        .unwrap_or(0);

    let percentile_keys: Vec<String> = app
        .timing_functions
        .percentiles
        .iter()
        .map(|p| hotpath::format_percentile_key(*p))
        .collect();

    let rows = entries.iter().map(|func| {
        let short_name = hotpath::shorten_function_name(&func.name);

        let mut cells = vec![
            Cell::from(short_name),
            Cell::from(hotpath::format_count_display(func.calls)),
            Cell::from(func.avg.clone()),
        ];

        for key in &percentile_keys {
            let value = func.percentiles.get(key).cloned().unwrap_or_default();
            cells.push(Cell::from(value));
        }

        cells.push(Cell::from(func.total.clone()));
        cells.push(Cell::from(func.percent_total.clone()));

        Row::new(cells)
    });

    let show_logs = app.show_function_logs;
    let focus = app.functions_focus;

    let num_percentiles = app.timing_functions.percentiles.len();

    let function_pct: u16 = 35;
    let remaining_pct: u16 = 100 - function_pct;
    let num_other_cols = (4 + num_percentiles) as u16; // Calls, Avg, P95s, Total, % Total
    let col_pct: u16 = remaining_pct / num_other_cols;

    let table = Table::new(
        rows,
        vec![Constraint::Percentage(function_pct)] // Function
            .into_iter()
            .chain(vec![
                Constraint::Percentage(col_pct), // Calls
                Constraint::Percentage(col_pct), // Avg
            ])
            .chain((0..num_percentiles).map(|_| Constraint::Percentage(col_pct))) // P95, etc
            .chain(vec![
                Constraint::Percentage(col_pct), // Total
                Constraint::Percentage(col_pct), // % Total
            ])
            .collect::<Vec<_>>(),
    )
    .header(header)
    .block(if show_logs {
        let border_set = if focus == FunctionsFocus::Functions {
            border::THICK
        } else {
            border::PLAIN
        };
        Block::bordered()
            .title(format!(" [{}/{}] ", function_position, total_functions))
            .title(Span::styled(title, common_styles::TITLE_STYLE_YELLOW))
            .border_set(border_set)
            .border_style(if focus == FunctionsFocus::Functions {
                Style::default()
            } else {
                common_styles::UNFOCUSED_BORDER_STYLE
            })
    } else {
        Block::bordered()
            .title(format!(" [{}/{}] ", function_position, total_functions))
            .title(Span::styled(title, common_styles::TITLE_STYLE_YELLOW))
            .border_set(border::THICK)
    })
    .row_highlight_style(common_styles::SELECTED_ROW_STYLE)
    .highlight_symbol(">> ");

    frame.render_stateful_widget(table, area, &mut app.timing_table_state);
}
