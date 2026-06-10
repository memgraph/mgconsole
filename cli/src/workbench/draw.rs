//! The ratatui draw edge: render the [`WorkbenchState`] into a frame.
//!
//! A thin, mechanical projection of the state — the stacked editor / results /
//! status layout (issue 01). It reads state and never mutates it, the analogue
//! of the REPL's rustyline `Helper`. Behaviour lives in the reducer; this is
//! kept thin and covered by a `TestBackend` smoke render rather than by detail.

use std::collections::BTreeMap;

use mgconsole_core::render;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;
use mgconsole_core::Value;

use super::highlight;
use super::plan::Plan;
use super::schema::Schema;
use super::state::{
    Completion, CurrentResult, DrawerKind, ExportPrompt, Focus, RunState, WorkbenchState,
};
use ratatui::text::Span;

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
    // When a side drawer is open it takes a fixed column on the right; the rest
    // is the main editor/results/status stack.
    let main_area = match state.drawer {
        Some(kind) => {
            let columns = Layout::horizontal([Constraint::Min(0), Constraint::Length(32)])
                .split(frame.area());
            match kind {
                DrawerKind::Schema => draw_sidebar(frame, columns[1], state.schema.as_ref()),
                DrawerKind::Params => draw_params(frame, columns[1], &state.params),
                DrawerKind::Summary => {
                    draw_summary(frame, columns[1], state.shown(), state.config.verbose);
                }
            }
            columns[0]
        }
        None => frame.area(),
    };

    let areas = Layout::vertical([
        Constraint::Percentage(state.config.editor_percent),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(main_area);
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
    let editor_cursor = draw_editor(frame, editor_inner, state, editor_focused);

    // Results pane: a native table with a pinned header and a lazily-rendered
    // visible window, or just a summary line for a result with no columns.
    let results_focused = matches!(state.focus, Focus::Results);
    let title = match state.shown() {
        Some(result) if result.plan.is_some() => {
            format!("Results [{}/{}] (plan)", state.view + 1, state.history.len())
        }
        Some(result) => format!(
            "Results [{}/{}] ({} row{}{}{})",
            state.view + 1,
            state.history.len(),
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
    if let Some(result) = state.shown() {
        draw_result(frame, results_inner, result, results_focused);
    }

    // Status bar: the transient message, then the keybind hints.
    frame.render_widget(Paragraph::new(status_text(state)), status_area);

    // Overlays, drawn last so they sit above the panes (at most one is open).
    if let Some(value) = state.detail.as_ref() {
        draw_detail(frame, value, state.detail_scroll);
    }
    if let Some(prompt) = state.export.as_ref() {
        draw_export(frame, prompt);
    }
    // Completion popup (slice 11), anchored just below the editor cursor.
    if let Some(completion) = state.completion.as_ref() {
        let (cx, cy) = editor_cursor;
        draw_completion(frame, completion, cx, cy);
    }
}

/// Draw the completion popup: a small list of candidates anchored below the
/// cursor, the selected one highlighted, with a window that keeps the selection
/// visible (slice 11).
fn draw_completion(frame: &mut Frame, completion: &Completion, cursor_x: u16, cursor_y: u16) {
    const MAX_VISIBLE: usize = 8;
    let visible = completion.candidates.len().min(MAX_VISIBLE);
    let width = completion
        .candidates
        .iter()
        .map(String::len)
        .max()
        .unwrap_or(8)
        .clamp(8, 40) as u16
        + 2;
    let height = visible as u16 + 2;
    let frame_area = frame.area();
    let x = cursor_x.min(frame_area.width.saturating_sub(width));
    let y = (cursor_y + 1).min(frame_area.height.saturating_sub(height));
    let area = ratatui::layout::Rect::new(x, y, width, height);

    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Window the list so the selected candidate stays visible.
    let start = if completion.selected >= MAX_VISIBLE {
        completion.selected + 1 - MAX_VISIBLE
    } else {
        0
    };
    let lines: Vec<Line> = completion
        .candidates
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(i, candidate)| {
            let style = if i == completion.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            Line::styled(candidate.clone(), style)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draw the schema sidebar (slice 13): the labels, relationship types, and
/// property keys the database holds, each in its own section. Refresh is shared
/// with completion (Ctrl-R); the drawer is only opened when a Schema exists.
fn draw_sidebar(frame: &mut Frame, area: Rect, schema: Option<&Schema>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Schema (Ctrl-R refresh · Ctrl-B close)");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    if let Some(schema) = schema {
        section(&mut lines, "Labels", &schema.labels);
        section(&mut lines, "Relationship types", &schema.rel_types);
        section(&mut lines, "Property keys", &schema.property_keys);
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Draw the parameters drawer (slice 16): the current `:param` set, one
/// `$name = value` per line, reusing the REPL's listing.
fn draw_params(frame: &mut Frame, area: Rect, params: &BTreeMap<String, Value>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Parameters (Ctrl-P close)");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(crate::repl::format_params(params)).wrap(Wrap { trim: false }),
        inner,
    );
}

/// Draw the summary drawer (slice 18): the shown result's notifications, update
/// statistics, and — when `--verbose-execution-info` is set — the per-query
/// execution info. Reuses the Core's `Summary`/`Notification` types; distinct
/// from the result data.
fn draw_summary(frame: &mut Frame, area: Rect, result: Option<&CurrentResult>, verbose: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Summary (Ctrl-Y close)");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    match result.and_then(|result| result.summary.as_ref()) {
        Some(summary) => {
            if !summary.notifications.is_empty() {
                lines.push(bold_line("Notifications"));
                for note in &summary.notifications {
                    lines.push(Line::from(format!("  [{}] {}", note.severity, note.title)));
                    if !note.description.is_empty() {
                        lines.push(Line::from(format!("    {}", note.description)));
                    }
                }
                lines.push(Line::from(""));
            }
            if !summary.stats.is_empty() {
                lines.push(bold_line("Statistics"));
                for (key, value) in &summary.stats {
                    lines.push(Line::from(format!("  {key}: {}", render::tabular(value))));
                }
                lines.push(Line::from(""));
            }
            if verbose {
                let info = summary.execution_info();
                lines.push(bold_line("Execution info"));
                push_timing(&mut lines, "cost estimate", info.cost_estimate);
                push_timing(&mut lines, "parsing", info.parsing_time);
                push_timing(&mut lines, "planning", info.planning_time);
                push_timing(&mut lines, "execution", info.plan_execution_time);
            }
            if lines.is_empty() {
                lines.push(Line::from("(no notifications or statistics)"));
            }
        }
        None => lines.push(Line::from("(run a query to see its summary)")),
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// A bold heading line for a drawer section.
fn bold_line(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        title.to_string(),
        Style::default().add_modifier(Modifier::BOLD),
    ))
}

/// Append an execution-info value line when the server reported it.
fn push_timing(lines: &mut Vec<Line<'static>>, label: &str, value: Option<f64>) {
    if let Some(value) = value {
        lines.push(Line::from(format!("  {label}: {value}")));
    }
}

/// Append a titled section of names to the sidebar lines.
fn section(lines: &mut Vec<Line<'static>>, title: &str, items: &[String]) {
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    if items.is_empty() {
        lines.push(Line::from("  (none)"));
    } else {
        for item in items {
            lines.push(Line::from(format!("  {item}")));
        }
    }
    lines.push(Line::from(""));
}

/// Draw the export prompt (slice 09): the chosen format and the destination path
/// being typed, in a centred box.
fn draw_export(frame: &mut Frame, prompt: &ExportPrompt) {
    let area = centered_rect(frame.area(), 60, 30);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title("Export result (Tab: format · Enter: write · Esc: cancel)");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let text = format!("Format: {}\nPath:   {}_", prompt.format.as_str(), prompt.path);
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
}

/// Draw the cell-detail overlay: a centred box showing the full Value rendered by
/// the Core's per-Value renderer, wrapped and scrollable so a node/path/map/list
/// is readable in full (slice 08).
fn draw_detail(frame: &mut Frame, value: &Value, scroll: u16) {
    let area = centered_rect(frame.area(), 70, 60);
    frame.render_widget(Clear, area); // clear what's beneath the overlay
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title("Cell detail (↑/↓ scroll · Esc/Enter close)");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(render::tabular(value))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );
}

/// Render a query plan as a navigable, collapsible operator tree (slice 14):
/// each visible line indented by its depth, a ▸/▾ marker on a node with a
/// subtree, the selected line highlighted.
fn draw_plan(frame: &mut Frame, area: Rect, plan: &Plan, focused: bool) {
    let lines: Vec<Line> = plan
        .visible()
        .into_iter()
        .map(|index| {
            let node = &plan.lines[index];
            let marker = if plan.has_children(index) {
                if node.collapsed {
                    "▸ "
                } else {
                    "▾ "
                }
            } else {
                "  "
            };
            let operator = format!("{}{marker}{}", " ".repeat(node.depth), node.operator);
            let row_style = if focused && index == plan.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            // PROFILE annotation (hits/time) trails the operator, dimmed.
            match &node.annotation {
                Some(annotation) => Line::from(vec![
                    Span::styled(format!("{operator}  "), row_style),
                    Span::styled(
                        annotation.clone(),
                        row_style.add_modifier(Modifier::DIM),
                    ),
                ]),
                None => Line::styled(operator, row_style),
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// A rectangle centred in `area`, sized to `pct_x` × `pct_y` percent of it.
fn centered_rect(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let rows = Layout::vertical([
        Constraint::Percentage((100 - pct_y) / 2),
        Constraint::Percentage(pct_y),
        Constraint::Percentage((100 - pct_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - pct_x) / 2),
        Constraint::Percentage(pct_x),
        Constraint::Percentage((100 - pct_x) / 2),
    ])
    .split(rows[1])[1]
}

/// Render the editor into `area`: each visible line highlighted by token
/// category (slice 05), the visible window following the cursor so a long query
/// stays editable. When focused, the terminal cursor is placed at the editor
/// cursor. (tui-textarea has no per-token styling, so the workbench renders the
/// lines itself and uses the widget only as the edit/cursor model.)
fn draw_editor(frame: &mut Frame, area: Rect, state: &WorkbenchState, focused: bool) -> (u16, u16) {
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
        .map(|line| highlight::highlight_line(line, state.color, &state.palette))
        .collect();
    frame.render_widget(Paragraph::new(visible), area);
    let x = area.x + (cursor_col as u16).min(area.width.saturating_sub(1));
    let y = area.y + (cursor_row - scroll) as u16;
    if focused {
        frame.set_cursor_position((x, y));
    }
    (x, y)
}

/// Render a streamed result into `area`: a table with a pinned header and only
/// the visible window of rows drawn (so a huge result is cheap), the selected
/// cell highlighted. A result with no columns shows nothing here (its summary is
/// in the status bar).
fn draw_result(frame: &mut Frame, area: Rect, result: &CurrentResult, focused: bool) {
    // An EXPLAIN/PROFILE result renders as an operator tree, not a table.
    if let Some(plan) = &result.plan {
        draw_plan(frame, area, plan, focused);
        return;
    }
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

/// Compose the status line: the active profile (issue 03), then the current
/// message (if any), then the keybind hints, including the universal newline key
/// (issue 01 AC).
fn status_text(state: &WorkbenchState) -> String {
    use std::fmt::Write as _;
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
    // Stable prefixes that survive transient messages: the endpoint/profile
    // (issues 03/07) and the read-only guard (issue 04).
    let mut prefix = String::new();
    if !state.endpoint.is_empty() {
        write!(prefix, "{}", state.endpoint).unwrap();
        if let Some(db) = &state.database {
            write!(prefix, "/{db}").unwrap();
        }
    }
    if let Some(name) = &state.profile {
        if !prefix.is_empty() {
            prefix.push(' ');
        }
        write!(prefix, "({name})").unwrap();
    }
    if state.read_only {
        if !prefix.is_empty() {
            prefix.push(' ');
        }
        prefix.push_str("[read-only]");
    }
    let tx_marker = match state.tx {
        mgconsole_core::TransactionState::Auto => "",
        mgconsole_core::TransactionState::Open => "[tx]",
        mgconsole_core::TransactionState::Failed => "[tx failed]",
    };
    if !tx_marker.is_empty() {
        if !prefix.is_empty() {
            prefix.push(' ');
        }
        prefix.push_str(tx_marker);
    }
    if state.watch.is_some() {
        if !prefix.is_empty() {
            prefix.push(' ');
        }
        prefix.push_str("[watch]");
    }
    [prefix, message, hints]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("  │  ")
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
    fn the_status_bar_shows_the_endpoint_and_profile() {
        let config = WorkbenchConfig {
            profile: Some("prod".to_string()),
            endpoint: "db:7688".to_string(),
            ..WorkbenchConfig::default()
        };
        let mut state = WorkbenchState::new(config, true);
        let rendered = render(&mut state);
        assert!(rendered.contains("db:7688"), "status bar names the endpoint");
        assert!(rendered.contains("(prod)"), "status bar names the profile");
    }

    #[test]
    fn the_status_bar_omits_the_profile_when_none_is_selected() {
        let mut state = WorkbenchState::new(WorkbenchConfig::default(), true);
        let rendered = render(&mut state);
        assert!(!rendered.contains('('), "no stale profile marker: {rendered}");
    }

    #[test]
    fn the_status_bar_shows_the_read_only_marker() {
        let config = WorkbenchConfig {
            read_only: true,
            ..WorkbenchConfig::default()
        };
        let mut state = WorkbenchState::new(config, true);
        let rendered = render(&mut state);
        assert!(rendered.contains("[read-only]"), "status bar marks read-only");
    }

    #[test]
    fn the_status_bar_shows_the_transaction_marker() {
        let mut state = WorkbenchState::new(WorkbenchConfig::default(), true);
        state.tx = mgconsole_core::TransactionState::Open;
        assert!(render(&mut state).contains("[tx]"), "open tx marked");
        state.tx = mgconsole_core::TransactionState::Failed;
        assert!(render(&mut state).contains("[tx failed]"), "failed tx marked");
    }

    #[test]
    fn renders_a_plan_as_an_operator_tree() {
        use crate::workbench::plan::{Plan, PlanLine};
        let mut state = WorkbenchState::new(WorkbenchConfig::default(), true);
        let mut result = CurrentResult::new("EXPLAIN ...".to_string(), vec!["QUERY PLAN".to_string()]);
        result.plan = Some(Plan {
            lines: vec![
                PlanLine { depth: 0, operator: "Produce {n}".into(), collapsed: false, annotation: None },
                PlanLine { depth: 2, operator: "ScanAll (n)".into(), collapsed: false, annotation: None },
            ],
            selected: 0,
        });
        state.history.push(result);
        let rendered = render(&mut state);
        assert!(rendered.contains("Produce {n}"), "operator drawn");
        assert!(rendered.contains("ScanAll (n)"), "child operator drawn");
        assert!(rendered.contains("plan"), "title marks plan mode");
        assert!(rendered.contains('▾'), "an expandable node shows a marker");
    }

    #[test]
    fn renders_a_profile_plan_with_annotations() {
        use crate::workbench::plan::{Plan, PlanLine};
        let mut state = WorkbenchState::new(WorkbenchConfig::default(), true);
        let mut result = CurrentResult::new("PROFILE ...".to_string(), vec!["OPERATOR".to_string()]);
        result.plan = Some(Plan {
            lines: vec![PlanLine {
                depth: 0,
                operator: "Produce {n}".into(),
                collapsed: false,
                annotation: Some("2 hits".to_string()),
            }],
            selected: 0,
        });
        state.history.push(result);
        let rendered = render(&mut state);
        assert!(rendered.contains("Produce {n}"), "operator drawn");
        assert!(rendered.contains("2 hits"), "annotation drawn");
    }

    #[test]
    fn renders_the_summary_drawer_with_notifications_and_stats() {
        use mgconsole_core::{Notification, Summary, Value};
        let mut state = WorkbenchState::new(WorkbenchConfig::default(), true);
        let mut result = CurrentResult::new("CREATE ...".to_string(), vec![]);
        result.summary = Some(Summary {
            notifications: vec![Notification {
                code: "Hint".into(),
                title: "Add an index".into(),
                description: String::new(),
                severity: "INFORMATION".into(),
            }],
            stats: std::collections::BTreeMap::from([(
                "nodes-created".to_string(),
                Value::Integer(1),
            )]),
            ..Summary::default()
        });
        state.history.push(result);
        state.drawer = Some(DrawerKind::Summary);
        let rendered = render(&mut state);
        assert!(rendered.contains("Notifications"), "section header");
        assert!(rendered.contains("Add an index"), "notification title");
        assert!(rendered.contains("nodes-created"), "update statistic");
    }

    #[test]
    fn renders_the_schema_sidebar_when_open() {
        use crate::workbench::schema::Schema;
        let mut state = WorkbenchState::new(WorkbenchConfig::default(), true);
        state.schema = Some(Schema {
            labels: vec!["Person".to_string()],
            rel_types: vec!["WORKS_AT".to_string()],
            property_keys: vec!["name".to_string()],
        });
        state.drawer = Some(DrawerKind::Schema);
        let rendered = render(&mut state);
        assert!(rendered.contains("Labels"), "section header drawn");
        assert!(rendered.contains("Person"), "a label listed");
        assert!(rendered.contains("WORKS_AT"), "a relationship type listed");
    }

    #[test]
    fn renders_a_result_table_with_header_rows_and_a_live_count() {
        use mgconsole_core::{Record, Value};
        let mut state = WorkbenchState::new(WorkbenchConfig::default(), true);
        state.history.push(CurrentResult {
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
