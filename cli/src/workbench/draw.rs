//! The ratatui draw edge: render the [`WorkbenchState`] into a frame.
//!
//! A thin, mechanical projection of the state — the stacked editor / results /
//! status layout (issue 01). It reads state and never mutates it, the analogue
//! of the REPL's rustyline `Helper`. Behaviour lives in the reducer; this is
//! kept thin and covered by a `TestBackend` smoke render rather than by detail.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::state::{Focus, WorkbenchState};

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

    // Results pane: empty until slice 03 streams rows into it.
    let results_focused = matches!(state.focus, Focus::Results);
    let results_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style(results_focused))
        .title("Results");
    frame.render_widget(results_block, results_area);

    // Status bar: the transient message, then the keybind hints.
    frame.render_widget(Paragraph::new(status_text(state)), status_area);
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
