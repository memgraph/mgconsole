//! The workbench reducer: `update(&mut state, event) -> Vec<Effect>`.
//!
//! Pure and terminal-free, the analogue of the REPL's `run_loop` body. It
//! mutates the [`WorkbenchState`] in place (D3: avoids cloning the result rows
//! every keystroke) and returns the IO to perform as [`Effect`]s. Slice 01
//! handles the shell gestures — editing, the newline key, focus, submit, and
//! quit; query execution and its lifecycle events arrive in slice 02. Driven in
//! tests by hand-built [`Event`]s, with no terminal and no database.

use std::time::Duration;

use mgconsole_core::{Error, QueryAssembler, Record, Summary};

use crate::repl::{format_summary, meta_command, MetaCommand};

use super::effect::Effect;
use super::event::{Event, Key, KeyCode};
use super::state::{CurrentResult, Focus, RunState, WorkbenchState};

/// Apply one event to the state, returning the effects to perform.
// Events are consumed by value: slice 02's lifecycle events carry owned data
// (a Record, a Summary, an Error) the reducer takes ownership of. Today's
// Key/Resize payloads are Copy, so the lint is transitional to this slice.
#[allow(clippy::needless_pass_by_value)]
pub fn update(state: &mut WorkbenchState, event: Event) -> Vec<Effect> {
    match event {
        // The draw re-reads the terminal size each frame, so a resize needs no
        // state change today; the event exists for layouts that will.
        Event::Resize(_, _) => Vec::new(),
        Event::Key(key) => update_key(state, key),
        Event::QueryStarted { id, header } => {
            on_started(state, id, header);
            Vec::new()
        }
        Event::RecordArrived { id, record } => {
            on_record(state, id, record);
            Vec::new()
        }
        Event::QueryCompleted {
            id,
            summary,
            elapsed,
        } => on_completed(state, id, summary, elapsed),
        Event::QueryFailed { id, error } => on_failed(state, id, &error),
    }
}

/// Whether `id` is the query currently in flight (so its events are live, not
/// stragglers from a superseded query — the one-live-result guard, ADR 0005).
fn is_current(state: &WorkbenchState, id: u64) -> bool {
    matches!(state.run, RunState::Running { id: running } if running == id)
}

/// A query began: start a fresh result for its rows to stream into.
fn on_started(state: &mut WorkbenchState, id: u64, header: Vec<String>) {
    if !is_current(state, id) {
        return;
    }
    state.result = Some(CurrentResult::new(header));
}

/// A record streamed in: append it to the live result, up to the row-cap
/// backstop (beyond which rows are dropped and the result marked truncated —
/// a memory guard, not a usability limit).
fn on_record(state: &mut WorkbenchState, id: u64, record: Record) {
    if !is_current(state, id) {
        return;
    }
    if let Some(result) = state.result.as_mut() {
        if result.rows.len() < state.config.row_cap {
            result.rows.push(record);
        } else {
            result.truncated = true;
        }
    }
}

/// The query drained: show its row count and elapsed time (reusing the REPL's
/// summary line), keep its trailing summary, then run the next pending statement
/// or fall idle.
fn on_completed(
    state: &mut WorkbenchState,
    id: u64,
    summary: Summary,
    elapsed: Duration,
) -> Vec<Effect> {
    if !is_current(state, id) {
        return Vec::new();
    }
    let rows = state.result.as_ref().map_or(0, |result| result.rows.len());
    state.status.message = format_summary(rows, elapsed);
    if let Some(result) = state.result.as_mut() {
        result.summary = Some(summary);
    }
    advance(state)
}

/// The query failed: surface the error without losing the Session (ADR 0005),
/// then continue with the next pending statement (as the REPL does mid-batch).
fn on_failed(state: &mut WorkbenchState, id: u64, error: &Error) -> Vec<Effect> {
    if !is_current(state, id) {
        return Vec::new();
    }
    state.status.message = format!("error: {error}");
    advance(state)
}

/// Run the next pending statement of a multi-statement submit, or fall idle when
/// the batch is done.
fn advance(state: &mut WorkbenchState) -> Vec<Effect> {
    if let Some(next) = state.pending.pop_front() {
        start_query(state, next)
    } else {
        state.run = RunState::Idle;
        Vec::new()
    }
}

/// Begin running `query`: stamp it with a fresh id, mark the Session busy, and
/// emit the run effect with the bound parameters.
fn start_query(state: &mut WorkbenchState, query: String) -> Vec<Effect> {
    let id = state.next_id;
    state.next_id += 1;
    state.run = RunState::Running { id };
    state.status.message = "running…".to_string();
    vec![Effect::RunQuery {
        id,
        query,
        params: state.params.clone(),
    }]
}

fn update_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    // Quit gestures work from any pane (ADR 0010 AC: Esc / Ctrl-D leave).
    if key.code == KeyCode::Esc || (key.ctrl && key.code == KeyCode::Char('d')) {
        return vec![Effect::Quit];
    }
    match state.focus {
        Focus::Editor => editor_key(state, key),
        Focus::Results => results_key(state, key),
    }
}

/// Keys while the editor pane has focus.
fn editor_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    match key {
        // Tab cycles focus to the results pane.
        Key {
            code: KeyCode::Tab,
            ctrl: false,
            alt: false,
            ..
        } => {
            state.focus = Focus::Results;
            Vec::new()
        }
        // The universal newline gesture (slice 01): Alt+Enter or Ctrl+J insert a
        // newline; plain Enter (below) submits. Ctrl/Shift+Enter on capable
        // terminals is negotiated in slice 06.
        Key {
            code: KeyCode::Enter,
            alt: true,
            ..
        }
        | Key {
            code: KeyCode::Char('j'),
            ctrl: true,
            ..
        } => {
            state.editor.insert_newline();
            Vec::new()
        }
        // Plain Enter submits the buffer.
        Key {
            code: KeyCode::Enter,
            ctrl: false,
            alt: false,
            ..
        } => submit(state),
        // Everything else is ordinary editing, delegated to the editor widget.
        other => {
            state.editor.edit(other);
            Vec::new()
        }
    }
}

/// Keys while the results pane has focus: navigate the table (the cursor is
/// clamped to the rows/columns that exist), or Tab back to the editor. Paging
/// uses the viewport height the draw last cached. Only the visible window is
/// ever rendered, so navigation over a huge result is cheap.
fn results_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    if key.code == KeyCode::Tab {
        state.focus = Focus::Editor;
        return Vec::new();
    }
    let page = state.viewport_rows.max(1);
    if let Some(result) = state.result.as_mut() {
        let last_row = result.rows.len().saturating_sub(1);
        let last_col = result.header.len().saturating_sub(1);
        match key.code {
            KeyCode::Up => result.selected_row = result.selected_row.saturating_sub(1),
            KeyCode::Down => result.selected_row = (result.selected_row + 1).min(last_row),
            KeyCode::PageUp => result.selected_row = result.selected_row.saturating_sub(page),
            KeyCode::PageDown => result.selected_row = (result.selected_row + page).min(last_row),
            KeyCode::Home => result.selected_row = 0,
            KeyCode::End => result.selected_row = last_row,
            KeyCode::Left => result.selected_col = result.selected_col.saturating_sub(1),
            KeyCode::Right => result.selected_col = (result.selected_col + 1).min(last_col),
            _ => {}
        }
        // Keep the selected row within the visible window.
        if result.selected_row < result.scroll {
            result.scroll = result.selected_row;
        } else if result.selected_row >= result.scroll + page {
            result.scroll = result.selected_row + 1 - page;
        }
    }
    Vec::new()
}

/// Handle a submit (plain Enter): split the buffer into statements and run the
/// first, queuing the rest to run sequentially on the one Session. A `:quit`/
/// `:exit` buffer leaves the workbench (reusing the REPL's meta parser), even
/// while a query runs. Submitting while a query is in flight is refused with a
/// "session busy" status (one-live-result, ADR 0005). The editor keeps its text
/// so the query can be edited and re-run.
fn submit(state: &mut WorkbenchState) -> Vec<Effect> {
    let buffer = state.editor.buffer();
    if matches!(meta_command(buffer.trim()), Some(MetaCommand::Quit)) {
        return vec![Effect::Quit];
    }
    if matches!(state.run, RunState::Running { .. }) {
        state.status.message = "session busy — cancel first".to_string();
        return Vec::new();
    }
    let mut statements = split_statements(&buffer).into_iter();
    let Some(first) = statements.next() else {
        return Vec::new(); // a blank buffer submits nothing
    };
    state.pending = statements.collect();
    start_query(state, first)
}

/// Split the editor buffer into complete statements using the Core's
/// [`QueryAssembler`] (the same `;`-aware splitting the REPL/import paths use).
/// A trailing statement with no `;` is run as a single query, decoupling submit
/// from the REPL's `;`-completeness rule (PRD).
fn split_statements(buffer: &str) -> Vec<String> {
    let mut assembler = QueryAssembler::new();
    // The trailing newline lets a line comment close before the buffer ends.
    let mut statements = assembler.push(&format!("{buffer}\n"));
    if assembler.has_pending() {
        let pending = assembler.pending().trim();
        if !pending.is_empty() {
            statements.push(pending.to_string());
        }
    }
    statements
        .into_iter()
        .filter(|statement| !statement.trim().is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::state::WorkbenchConfig;
    use std::collections::BTreeMap;
    use mgconsole_core::error::QueryError;
    use mgconsole_core::Value;

    fn wb() -> WorkbenchState {
        WorkbenchState::new(WorkbenchConfig::default(), true)
    }

    /// Submit `query` and return the id the reducer stamped on it.
    fn submit_query(state: &mut WorkbenchState, query: &str) -> u64 {
        type_str(state, query);
        let effects = update(state, Event::Key(Key::plain(KeyCode::Enter)));
        match effects.first() {
            Some(Effect::RunQuery { id, .. }) => *id,
            other => panic!("expected a RunQuery effect, got {other:?}"),
        }
    }

    fn one_row() -> Record {
        Record::new(vec![Value::Integer(1)])
    }

    /// Type a string into the editor, one character event at a time.
    fn type_str(state: &mut WorkbenchState, text: &str) {
        for c in text.chars() {
            update(state, Event::Key(Key::char(c)));
        }
    }

    #[test]
    fn typing_characters_builds_the_editor_buffer() {
        let mut s = wb();
        type_str(&mut s, "MATCH (n)");
        assert_eq!(s.editor.buffer(), "MATCH (n)");
    }

    #[test]
    fn backspace_deletes_the_last_character() {
        let mut s = wb();
        type_str(&mut s, "RETURNx");
        update(&mut s, Event::Key(Key::plain(KeyCode::Backspace)));
        assert_eq!(s.editor.buffer(), "RETURN");
    }

    #[test]
    fn the_cursor_moves_freely_for_mid_buffer_editing() {
        // Type, jump to the line start, and insert — free cursor editing, the
        // win over a line REPL (AC 2).
        let mut s = wb();
        type_str(&mut s, "abc");
        update(&mut s, Event::Key(Key::plain(KeyCode::Home)));
        type_str(&mut s, "X");
        assert_eq!(s.editor.buffer(), "Xabc");
    }

    #[test]
    fn alt_enter_inserts_a_newline_without_submitting() {
        let mut s = wb();
        type_str(&mut s, "ab");
        let effects = update(&mut s, Event::Key(Key::alt(KeyCode::Enter)));
        type_str(&mut s, "cd");
        assert_eq!(s.editor.buffer(), "ab\ncd");
        assert!(effects.is_empty(), "a newline produces no effect");
    }

    #[test]
    fn ctrl_j_also_inserts_a_newline() {
        let mut s = wb();
        type_str(&mut s, "ab");
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('j'))));
        type_str(&mut s, "cd");
        assert_eq!(s.editor.buffer(), "ab\ncd");
    }

    #[test]
    fn plain_enter_runs_the_buffer_as_a_query() {
        let mut s = wb();
        type_str(&mut s, "RETURN 1;");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(
            effects,
            vec![Effect::RunQuery {
                id: 0,
                query: "RETURN 1".to_string(),
                params: BTreeMap::new(),
            }]
        );
        assert!(matches!(s.run, RunState::Running { id: 0 }));
        assert_eq!(s.editor.buffer(), "RETURN 1;", "the buffer is kept for re-run");
    }

    #[test]
    fn a_buffer_with_no_semicolon_runs_as_a_single_query() {
        let mut s = wb();
        type_str(&mut s, "RETURN 1");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(
            effects,
            vec![Effect::RunQuery {
                id: 0,
                query: "RETURN 1".to_string(),
                params: BTreeMap::new(),
            }]
        );
        assert!(s.pending.is_empty());
    }

    #[test]
    fn a_blank_buffer_submits_nothing() {
        let mut s = wb();
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty());
        assert!(matches!(s.run, RunState::Idle));
    }

    #[test]
    fn a_quit_meta_command_leaves_the_workbench() {
        let mut s = wb();
        type_str(&mut s, ":quit");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(effects, vec![Effect::Quit]);
    }

    #[test]
    fn esc_quits() {
        let mut s = wb();
        assert_eq!(
            update(&mut s, Event::Key(Key::plain(KeyCode::Esc))),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn ctrl_d_quits() {
        let mut s = wb();
        assert_eq!(
            update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('d')))),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn tab_cycles_focus_between_editor_and_results() {
        let mut s = wb();
        assert_eq!(s.focus, Focus::Editor);
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert_eq!(s.focus, Focus::Results);
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert_eq!(s.focus, Focus::Editor);
    }

    #[test]
    fn quit_works_from_the_results_pane_too() {
        let mut s = wb();
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab))); // focus results
        assert_eq!(
            update(&mut s, Event::Key(Key::plain(KeyCode::Esc))),
            vec![Effect::Quit]
        );
    }

    // --- execution spine (slice 02): lifecycle driven by hand-fed events -----

    #[test]
    fn streamed_records_append_and_the_summary_line_reports_count_and_time() {
        let mut s = wb();
        let id = submit_query(&mut s, "MATCH (n) RETURN n;");
        update(
            &mut s,
            Event::QueryStarted {
                id,
                header: vec!["n".to_string()],
            },
        );
        for _ in 0..3 {
            update(&mut s, Event::RecordArrived { id, record: one_row() });
        }
        let effects = update(
            &mut s,
            Event::QueryCompleted {
                id,
                summary: Summary::default(),
                elapsed: Duration::from_millis(5),
            },
        );
        assert!(effects.is_empty(), "no further query to run");
        assert!(matches!(s.run, RunState::Idle), "idle once drained");
        let result = s.result.as_ref().expect("a result");
        assert_eq!(result.rows.len(), 3, "all three rows appended");
        assert_eq!(s.status.message, "3 rows in set (0.005 sec)");
    }

    #[test]
    fn a_multi_statement_submit_runs_sequentially_on_one_session() {
        let mut s = wb();
        let id0 = submit_query(&mut s, "RETURN 1; RETURN 2;");
        assert_eq!(s.pending.len(), 1, "the second statement is queued");
        // Completing the first runs the second on the one Session.
        let effects = update(
            &mut s,
            Event::QueryCompleted {
                id: id0,
                summary: Summary::default(),
                elapsed: Duration::from_millis(1),
            },
        );
        assert_eq!(
            effects,
            vec![Effect::RunQuery {
                id: 1,
                query: "RETURN 2".to_string(),
                params: BTreeMap::new(),
            }]
        );
        assert!(s.pending.is_empty());
    }

    #[test]
    fn a_second_query_while_one_runs_is_refused_as_session_busy() {
        let mut s = wb();
        submit_query(&mut s, "MATCH (n) RETURN n;"); // now Running
        // Type and submit again while in flight.
        type_str(&mut s, "RETURN 2;");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty(), "the second query does not start");
        assert!(s.status.message.contains("busy"), "status: {}", s.status.message);
    }

    #[test]
    fn a_query_error_is_surfaced_and_the_session_survives() {
        let mut s = wb();
        let id = submit_query(&mut s, "BAD;");
        let boom = Error::Query(QueryError {
            code: "Memgraph.ClientError.MemgraphError.SyntaxError".to_string(),
            message: "bad cypher".to_string(),
        });
        update(&mut s, Event::QueryFailed { id, error: boom });
        assert!(s.status.message.contains("bad cypher"), "error surfaced");
        assert!(matches!(s.run, RunState::Idle), "session ready again");
        // The next query runs (the session was not lost).
        let next = submit_query(&mut s, "RETURN 1;");
        assert_eq!(next, 1);
    }

    // --- results table navigation (slice 03) --------------------------------

    /// A workbench focused on a results pane holding `n` two-column rows, with a
    /// viewport of `viewport` data rows.
    fn with_result(n: usize, viewport: usize) -> WorkbenchState {
        let mut s = wb();
        s.focus = Focus::Results;
        s.viewport_rows = viewport;
        s.result = Some(CurrentResult {
            header: vec!["a".to_string(), "b".to_string()],
            rows: (0..n).map(|_| one_row()).collect(),
            ..CurrentResult::default()
        });
        s
    }

    fn press(state: &mut WorkbenchState, code: KeyCode) {
        update(state, Event::Key(Key::plain(code)));
    }

    #[test]
    fn row_navigation_moves_and_clamps_within_bounds() {
        let mut s = with_result(5, 10);
        press(&mut s, KeyCode::Down);
        press(&mut s, KeyCode::Down);
        assert_eq!(s.result.as_ref().unwrap().selected_row, 2);
        for _ in 0..10 {
            press(&mut s, KeyCode::Down);
        }
        assert_eq!(s.result.as_ref().unwrap().selected_row, 4, "clamped to last row");
        for _ in 0..10 {
            press(&mut s, KeyCode::Up);
        }
        assert_eq!(s.result.as_ref().unwrap().selected_row, 0, "clamped to first row");
    }

    #[test]
    fn home_and_end_jump_to_first_and_last_row() {
        let mut s = with_result(50, 10);
        press(&mut s, KeyCode::End);
        assert_eq!(s.result.as_ref().unwrap().selected_row, 49);
        press(&mut s, KeyCode::Home);
        assert_eq!(s.result.as_ref().unwrap().selected_row, 0);
    }

    #[test]
    fn column_navigation_clamps_to_the_header_width() {
        let mut s = with_result(3, 10);
        press(&mut s, KeyCode::Right);
        assert_eq!(s.result.as_ref().unwrap().selected_col, 1);
        press(&mut s, KeyCode::Right); // only two columns, so clamp at 1
        assert_eq!(s.result.as_ref().unwrap().selected_col, 1);
        press(&mut s, KeyCode::Left);
        assert_eq!(s.result.as_ref().unwrap().selected_col, 0);
    }

    #[test]
    fn the_scroll_window_follows_the_selection() {
        // Viewport of 3 rows over 20: paging down scrolls the window so the
        // selection stays visible (only the visible window is ever drawn).
        let mut s = with_result(20, 3);
        press(&mut s, KeyCode::PageDown);
        let r = s.result.as_ref().unwrap();
        assert_eq!(r.selected_row, 3);
        assert_eq!(r.scroll, 1, "scrolled so row 3 sits at the window bottom");
    }

    #[test]
    fn rows_beyond_the_cap_are_dropped_and_the_result_is_marked_truncated() {
        let mut s = wb();
        s.config.row_cap = 2;
        let id = submit_query(&mut s, "MATCH (n) RETURN n;");
        update(&mut s, Event::QueryStarted { id, header: vec!["n".to_string()] });
        for _ in 0..5 {
            update(&mut s, Event::RecordArrived { id, record: one_row() });
        }
        let result = s.result.as_ref().unwrap();
        assert_eq!(result.rows.len(), 2, "held rows capped at the backstop");
        assert!(result.truncated, "truncation flagged");
    }

    #[test]
    fn lifecycle_events_from_a_superseded_query_are_ignored() {
        let mut s = wb();
        let id = submit_query(&mut s, "RETURN 1;");
        update(
            &mut s,
            Event::QueryStarted {
                id,
                header: vec!["x".to_string()],
            },
        );
        // A straggler record from a different (older) query id must not append.
        update(
            &mut s,
            Event::RecordArrived {
                id: id + 99,
                record: one_row(),
            },
        );
        assert_eq!(s.result.as_ref().expect("result").rows.len(), 0);
    }
}
