//! The ratatui draw edge: render the [`WorkbenchState`] into a frame.
//!
//! A thin, mechanical projection of the state — the stacked editor / results /
//! status layout (issue 01). It reads state and never mutates it, the analogue
//! of the REPL's rustyline `Helper`. Behaviour lives in the reducer; this is
//! kept thin and covered by a `TestBackend` smoke render rather than by detail.

use mgconsole_core::render;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::state::{CurrentResult, Focus, WorkbenchState};

/// Border colour for a focused pane vs. an unfocused one.
fn border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}

/// Render the whole workbench: editor on top, results below, a one-line status
/// bar at the bottom.
pub fn draw(frame: &mut Frame, state: &WorkbenchState) {
    let areas = Layout::vertical([
        Constraint::Percentage(state.config.editor_percent),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(frame.area());
    let (editor_area, results_area, status_area) = (areas[0], areas[1], areas[2]);

    // Editor pane: a bordered block with the query editor rendered inside it.
    let editor_focused = matches!(state.focus, Focus::Editor);
    let editor_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(editor_focused))
        .title("Query");
    let editor_inner = editor_block.inner(editor_area);
    frame.render_widget(editor_block, editor_area);
    frame.render_widget(state.editor.textarea(), editor_inner);

    // Results pane: a minimal text view of the streamed result (slice 02). The
    // navigable, scrollable table with a pinned header arrives in slice 03.
    let results_focused = matches!(state.focus, Focus::Results);
    let results_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(results_focused))
        .title("Results");
    let results_inner = results_block.inner(results_area);
    frame.render_widget(results_block, results_area);
    if let Some(result) = state.result.as_ref() {
        frame.render_widget(
            Paragraph::new(result_text(result, results_inner.height as usize)),
            results_inner,
        );
    }

    // Status bar: the transient message, then the keybind hints.
    frame.render_widget(Paragraph::new(status_text(state)), status_area);
}

/// A minimal text rendering of a streamed result: the header, then one line per
/// row of its cells (each rendered by the Core's per-Value renderer), capped to
/// the visible height. Slice 03 replaces this with a navigable ratatui table.
fn result_text(result: &CurrentResult, max_lines: usize) -> String {
    let mut lines = Vec::new();
    if !result.header.is_empty() {
        lines.push(result.header.join(" | "));
    }
    for row in &result.rows {
        if lines.len() >= max_lines {
            break;
        }
        let cells: Vec<String> = row.fields().iter().map(render::tabular).collect();
        lines.push(cells.join(" | "));
    }
    lines.join("\n")
}

/// Compose the status line: the current message (if any) followed by the
/// keybind hints, including the universal newline key (issue 01 AC).
fn status_text(state: &WorkbenchState) -> String {
    let hints = format!(
        "Enter: run · {}: newline · Tab: focus · Esc/Ctrl-D: quit",
        state.config.newline_hint
    );
    if state.status.message.is_empty() {
        hints
    } else {
        format!("{}  │  {hints}", state.status.message)
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
    #[test]
    fn renders_the_shell_layout_with_the_newline_hint() {
        let state = WorkbenchState::new(WorkbenchConfig::default(), true);
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("test backend");
        terminal
            .draw(|frame| draw(frame, &state))
            .expect("draw succeeds");
        let buffer = terminal.backend().buffer().clone();
        let rendered: String = buffer.content().iter().map(ratatui::buffer::Cell::symbol).collect();
        assert!(rendered.contains("Query"), "editor pane titled");
        assert!(rendered.contains("Results"), "results pane titled");
        assert!(rendered.contains("Alt+Enter"), "newline key surfaced in the hint");
    }
}
