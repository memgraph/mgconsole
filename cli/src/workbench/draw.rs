//! The ratatui draw edge: render the [`WorkbenchState`] into a frame.
//!
//! A thin, mechanical projection of the state — the stacked editor / results /
//! status layout (issue 01). It reads state and never mutates it, the analogue
//! of the REPL's rustyline `Helper`. Behaviour lives in the reducer; this is
//! kept thin and covered by a `TestBackend` smoke render rather than by detail.

use mgconsole_core::render;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use super::highlight;
use super::state::{CurrentResult, Focus, RunState, WorkbenchState};

/// Braille spinner frames for the running-query indicator (slice 07).
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Border colour for a focused pane vs. an unfocused one.
fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}

/// Render the whole workbench: editor on top, results below, a one-line status
/// bar at the bottom. Takes `&mut state` only to cache the results viewport
/// height it just laid out, so the reducer can page correctly; no logical state
/// is changed.
pub fn draw(frame: &mut Frame, state: &mut WorkbenchState) {
    let areas = Layout::vertical([
        Constraint::Percentage(state.config.editor_percent),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(frame.area());
    let (editor_area, results_area, status_area) = (areas[0], areas[1], areas[2]);

    // Editor pane: a bordered block with the query editor rendered inside it.
    // The lines are rendered with per-token syntax highlighting (slice 05), and
    // the terminal cursor is placed at the editor cursor when the pane is focused.
    let editor_focused = matches!(state.focus, Focus::Editor);
    let editor_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(editor_focused))
        .title("Query");
    let editor_inner = editor_block.inner(editor_area);
    frame.render_widget(editor_block, editor_area);
    draw_editor(frame, editor_inner, state, editor_focused);

    // Results pane: a native table with a pinned header and a lazily-rendered
    // visible window, or just a summary line for a result with no columns.
    let results_focused = matches!(state.focus, Focus::Results);
    let title = match state.result.as_ref() {
        Some(result) => format!(
            "Results ({} row{}{}{})",
            result.rows.len(),
            if result.rows.len() == 1 { "" } else { "s" },
            if result.truncated { ", truncated" } else { "" },
            if result.partial { ", partial" } else { "" }
        ),
        None => "Results".to_string(),
    };
    let results_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(results_focused))
        .title(title);
    let results_inner = results_block.inner(results_area);
    frame.render_widget(results_block, results_area);
    // Cache the data-row viewport (height minus the pinned header row).
    state.viewport_rows = results_inner.height.saturating_sub(1) as usize;
    if let Some(result) = state.result.as_ref() {
        draw_result(frame, results_inner, result, results_focused);
    }

    // Status bar: the transient message, then the keybind hints.
    frame.render_widget(Paragraph::new(status_text(state)), status_area);
}

/// Render the editor into `area`: each visible line highlighted by token
/// category (slice 05), the visible window following the cursor so a long query
/// stays editable. When focused, the terminal cursor is placed at the editor
/// cursor. (tui-textarea has no per-token styling, so the workbench renders the
/// lines itself and uses the widget only as the edit/cursor model.)
fn draw_editor(frame: &mut Frame, area: Rect, state: &WorkbenchState, focused: bool) {
    let height = area.height.max(1) as usize;
    let (cursor_row, cursor_col) = state.editor.cursor();
    // Keep the cursor line visible (pin to the bottom when scrolling past it).
    let scroll = if cursor_row >= height {
        cursor_row + 1 - height
    } else {
        0
    };
    let lines = state.editor.lines();
    let end = (scroll + height).min(lines.len());
    let visible: Vec<Line> = lines[scroll.min(lines.len())..end]
        .iter()
        .map(|line| highlight::highlight_line(line, state.color))
        .collect();
    frame.render_widget(Paragraph::new(visible), area);
    if focused {
        let x = area.x + (cursor_col as u16).min(area.width.saturating_sub(1));
        let y = area.y + (cursor_row - scroll) as u16;
        frame.set_cursor_position((x, y));
    }
}

/// Render a streamed result into `area`: a table with a pinned header and only
/// the visible window of rows drawn (so a huge result is cheap), the selected
/// cell highlighted. A result with no columns shows nothing here (its summary is
/// in the status bar).
fn draw_result(frame: &mut Frame, area: Rect, result: &CurrentResult, focused: bool) {
    if result.header.is_empty() {
        return;
    }
    let viewport = area.height.saturating_sub(1) as usize;
    let start = result.scroll.min(result.rows.len());
    let end = (start + viewport).min(result.rows.len());

    let rows = result.rows[start..end].iter().enumerate().map(|(offset, record)| {
        let absolute = start + offset;
        let cells = record.fields().iter().enumerate().map(|(col, value)| {
            let mut style = Style::default();
            // Highlight the selected cell when the pane is focused.
            if focused && absolute == result.selected_row && col == result.selected_col {
                style = style.add_modifier(Modifier::REVERSED);
            }
            Cell::from(render::tabular(value)).style(style)
        });
        Row::new(cells)
    });

    let widths: Vec<Constraint> = (0..result.header.len())
        .map(|_| Constraint::Ratio(1, result.header.len() as u32))
        .collect();
    let header = Row::new(
        result
            .header
            .iter()
            .map(|name| Cell::from(name.clone()).style(Style::default().add_modifier(Modifier::BOLD))),
    );
    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, area);
}

/// Compose the status line: the current message (if any) followed by the
/// keybind hints, including the universal newline key (issue 01 AC).
fn status_text(state: &WorkbenchState) -> String {
    let hints = format!(
        "Enter: run · {}: newline · Tab: focus · Ctrl-C: cancel · Esc/Ctrl-D: quit",
        state.config.newline_hint
    );
    // While a query is in flight, prefix a spinner to the running message.
    let message = if matches!(state.run, RunState::Running { .. }) {
        let frame = SPINNER[state.spinner % SPINNER.len()];
        format!("{frame} {}", state.status.message)
    } else {
        state.status.message.clone()
    };
    if message.is_empty() {
        hints
    } else {
        format!("{message}  │  {hints}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::state::WorkbenchConfig;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// A thin golden smoke render: the shell draws its three regions and the
    /// status hint names the newline key, with no terminal.
    fn render(state: &mut WorkbenchState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("test backend");
        terminal
            .draw(|frame| draw(frame, state))
            .expect("draw succeeds");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect()
    }

    #[test]
    fn renders_the_shell_layout_with_the_newline_hint() {
        let mut state = WorkbenchState::new(WorkbenchConfig::default(), true);
        let rendered = render(&mut state);
        assert!(rendered.contains("Query"), "editor pane titled");
        assert!(rendered.contains("Results"), "results pane titled");
        assert!(rendered.contains("Alt+Enter"), "newline key surfaced in the hint");
    }

    #[test]
    fn renders_a_result_table_with_header_rows_and_a_live_count() {
        use mgconsole_core::{Record, Value};
        let mut state = WorkbenchState::new(WorkbenchConfig::default(), true);
        state.result = Some(CurrentResult {
            header: vec!["name".to_string(), "age".to_string()],
            rows: vec![
                Record::new(vec![Value::String("Ada".into()), Value::Integer(36)]),
                Record::new(vec![Value::String("Bob".into()), Value::Integer(40)]),
            ],
            ..CurrentResult::default()
        });
        let rendered = render(&mut state);
        assert!(rendered.contains("name"), "header column drawn");
        assert!(rendered.contains("Ada"), "a row cell drawn");
        assert!(rendered.contains("2 rows"), "live row count in the title");
    }
}
