//! The workbench reducer: `update(&mut state, event) -> Vec<Effect>`.
//!
//! Pure and terminal-free, the analogue of the REPL's `run_loop` body. It
//! mutates the [`WorkbenchState`] in place (D3: avoids cloning the result rows
//! every keystroke) and returns the IO to perform as [`Effect`]s. Slice 01
//! handles the shell gestures — editing, the newline key, focus, submit, and
//! quit; query execution and its lifecycle events arrive in slice 02. Driven in
//! tests by hand-built [`Event`]s, with no terminal and no database.

use std::path::PathBuf;
use std::time::Duration;

use mgconsole_core::{Error, QueryAssembler, Record, Summary};

use crate::repl::{format_summary, meta_command, MetaCommand};
use crate::syntax::Completer;

use super::effect::Effect;
use super::event::{Event, Key, KeyCode};
use super::schema::{Schema, SchemaSource};
use super::state::{
    Completion, CurrentResult, DrawerKind, ExportPrompt, Focus, RunState, WorkbenchState,
};

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
        Event::ExportFinished(outcome) => {
            state.status.message = match outcome {
                Ok(path) => format!("exported to {}", path.display()),
                Err(message) => format!("export failed: {message}"),
            };
            Vec::new()
        }
        Event::SchemaLoaded(schema) => {
            set_schema(state, schema);
            Vec::new()
        }
        Event::Tick => {
            // Advance the running-query spinner; idle ticks change nothing.
            if matches!(state.run, RunState::Running { .. }) {
                state.spinner = state.spinner.wrapping_add(1);
            }
            Vec::new()
        }
    }
}

/// Whether `id` is the query currently in flight (so its events are live, not
/// stragglers from a superseded query — the one-live-result guard, ADR 0005).
fn is_current(state: &WorkbenchState, id: u64) -> bool {
    matches!(state.run, RunState::Running { id: running } if running == id)
}

/// A query began: push a fresh result onto the history stack for its rows to
/// stream into, and show it (auto-follow the running query).
fn on_started(state: &mut WorkbenchState, id: u64, header: Vec<String>) {
    if !is_current(state, id) {
        return;
    }
    state.history.push(CurrentResult::new(header));
    state.view = state.history.len() - 1;
}

/// A record streamed in: append it to the live (last) result, up to the row-cap
/// backstop (beyond which rows are dropped and the result marked truncated —
/// a memory guard, not a usability limit).
fn on_record(state: &mut WorkbenchState, id: u64, record: Record) {
    if !is_current(state, id) {
        return;
    }
    let cap = state.config.row_cap;
    if let Some(result) = state.live_mut() {
        if result.rows.len() < cap {
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
    let rows = state.history.last().map_or(0, |result| result.rows.len());
    state.status.message = format_summary(rows, elapsed);
    if let Some(result) = state.live_mut() {
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
    state.spinner = 0;
    state.status.message = "running…".to_string();
    vec![Effect::RunQuery {
        id,
        query,
        params: state.params.clone(),
    }]
}

/// Ctrl-C. While a query runs, cancel it: keep the rows already streamed on
/// screen but label the result partial, drop the rest of the batch, fall idle,
/// and emit [`Effect::Cancel`] (the edge aborts the task and RESETs the Session,
/// ADR 0005). When idle, abandon the typed buffer (the REPL's interrupt).
fn interrupt(state: &mut WorkbenchState) -> Vec<Effect> {
    if let RunState::Running { id } = state.run {
        let rows = state.history.last().map_or(0, |result| result.rows.len());
        if let Some(result) = state.live_mut() {
            result.partial = true;
        }
        state.pending.clear();
        state.run = RunState::Idle;
        state.status.message = format!(
            "cancelled — {rows} row{} (partial)",
            if rows == 1 { "" } else { "s" }
        );
        vec![Effect::Cancel { id }]
    } else {
        state.editor.clear();
        Vec::new()
    }
}

fn update_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    // Overlays capture keys while open — including Esc, so it dismisses the
    // overlay rather than quitting the workbench.
    if state.detail.is_some() {
        return detail_key(state, key);
    }
    if state.export.is_some() {
        return export_key(state, key);
    }
    // The completion popup captures its navigation keys (Esc included, so it
    // dismisses rather than quitting); any other key closes it and is handled as
    // ordinary input.
    if state.completion.is_some() {
        return completion_key(state, key);
    }
    // Quit gestures work from any pane (ADR 0010 AC: Esc / Ctrl-D leave).
    if key.code == KeyCode::Esc || (key.ctrl && key.code == KeyCode::Char('d')) {
        return vec![Effect::Quit];
    }
    // Ctrl-C from any pane: cancel an in-flight query (keep the session), or
    // abandon the typed buffer when idle (mirrors the REPL's interrupt).
    if key.ctrl && key.code == KeyCode::Char('c') {
        return interrupt(state);
    }
    // Ctrl-R refreshes the Schema (slice 12): re-fetch labels/types/keys.
    if key.ctrl && key.code == KeyCode::Char('r') {
        state.status.message = "refreshing schema…".to_string();
        return vec![Effect::FetchSchema];
    }
    // Ctrl-B toggles the schema sidebar (slice 13); it is unavailable when no
    // Schema was fetched (the feature is off — nothing to browse).
    if key.ctrl && key.code == KeyCode::Char('b') {
        toggle_schema_sidebar(state);
        return Vec::new();
    }
    match state.focus {
        Focus::Editor => editor_key(state, key),
        Focus::Results => results_key(state, key),
    }
}

/// Keys while the completion popup is open: cycle/insert/dismiss; any other key
/// closes the popup and is handled as ordinary input.
fn completion_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    match key.code {
        KeyCode::Esc => {
            state.completion = None;
            Vec::new()
        }
        KeyCode::Enter => {
            apply_completion(state);
            Vec::new()
        }
        KeyCode::Up => {
            cycle_completion(state, -1);
            Vec::new()
        }
        KeyCode::Down | KeyCode::Tab => {
            cycle_completion(state, 1);
            Vec::new()
        }
        _ => {
            state.completion = None;
            // The key that closed the popup is still ordinary editor input.
            match state.focus {
                Focus::Editor => editor_key(state, key),
                Focus::Results => results_key(state, key),
            }
        }
    }
}

/// Keys while the editor pane has focus.
fn editor_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    match key {
        // Tab triggers completion for the word under the cursor; with nothing to
        // complete (empty prefix or no candidates) it cycles focus to the results
        // pane instead (slice 11).
        Key {
            code: KeyCode::Tab,
            ctrl: false,
            alt: false,
            ..
        } => {
            open_completion(state);
            if state.completion.is_none() {
                state.focus = Focus::Results;
            }
            Vec::new()
        }
        // Newline gestures. The universal keys (everywhere): Alt+Enter, Ctrl+J.
        // On a keyboard-enhancement-capable terminal (negotiated at startup,
        // slice 06) Shift+Enter and Ctrl+Enter also arrive distinctly and insert
        // a newline; on terminals that cannot distinguish them, those events
        // never arrive and plain Enter (below) submits. Enter's meaning (submit)
        // is invariant across all terminals.
        Key {
            code: KeyCode::Enter,
            alt: true,
            ..
        }
        | Key {
            code: KeyCode::Enter,
            ctrl: true,
            ..
        }
        | Key {
            code: KeyCode::Enter,
            shift: true,
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
        // Plain Enter (no modifiers) submits the buffer.
        Key {
            code: KeyCode::Enter,
            ctrl: false,
            alt: false,
            shift: false,
        } => submit(state),
        // Everything else is ordinary editing, delegated to the editor widget.
        other => {
            state.editor.edit(other);
            Vec::new()
        }
    }
}

/// Toggle the schema sidebar drawer (slice 13). The drawer is unavailable when
/// no Schema is loaded — the feature is off, so there is nothing to browse.
fn toggle_schema_sidebar(state: &mut WorkbenchState) {
    if state.drawer == Some(DrawerKind::Schema) {
        state.drawer = None;
    } else if state.schema.is_some() {
        state.drawer = Some(DrawerKind::Schema);
    } else {
        state.status.message = "no schema to browse".to_string();
    }
}

/// Register the fetched Schema (slice 12): rebuild the completer as the static
/// vocabulary plus a schema source (so a re-fetch replaces, not stacks), and keep
/// the Schema for the sidebar. `None` (feature off) leaves static-only completion.
fn set_schema(state: &mut WorkbenchState, schema: Option<Schema>) {
    let mut completer = Completer::with_static_vocabulary();
    if let Some(schema) = &schema {
        completer.add_source(Box::new(SchemaSource::new(schema.all_names())));
        let total = schema.labels.len() + schema.rel_types.len() + schema.property_keys.len();
        state.status.message = format!("schema loaded ({total} names)");
    }
    state.completer = completer;
    state.schema = schema;
}

/// Open the completion popup for the word under the cursor, using the existing
/// `Completer` (reused unchanged). An empty prefix or no candidates leaves the
/// popup closed (the static completer already declines an empty prefix).
fn open_completion(state: &mut WorkbenchState) {
    let prefix = state.editor.word_under_cursor();
    let candidates = state.completer.candidates(&prefix);
    if candidates.is_empty() {
        return;
    }
    state.completion = Some(Completion {
        candidates,
        selected: 0,
        prefix_len: prefix.chars().count(),
    });
}

/// Move the completion selection by `delta`, wrapping around the candidate list.
fn cycle_completion(state: &mut WorkbenchState, delta: isize) {
    if let Some(completion) = state.completion.as_mut() {
        let len = completion.candidates.len() as isize;
        completion.selected = (completion.selected as isize + delta).rem_euclid(len) as usize;
    }
}

/// Insert the selected candidate, replacing the typed prefix, and close the popup.
fn apply_completion(state: &mut WorkbenchState) {
    if let Some(completion) = state.completion.take() {
        let candidate = completion.candidates[completion.selected].clone();
        state
            .editor
            .insert_completion(completion.prefix_len, &candidate);
    }
}

/// Keys while the results pane has focus: navigate the table (the cursor is
/// clamped to the rows/columns that exist), or Tab back to the editor. Paging
/// uses the viewport height the draw last cached. Only the visible window is
/// ever rendered, so navigation over a huge result is cheap.
fn results_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    match key.code {
        KeyCode::Tab => {
            state.focus = Focus::Editor;
            return Vec::new();
        }
        // Enter expands the selected cell into the detail overlay (slice 08).
        KeyCode::Enter => {
            open_detail(state);
            return Vec::new();
        }
        // 'e' opens the export prompt for the on-screen result (slice 09).
        KeyCode::Char('e') => {
            open_export(state);
            return Vec::new();
        }
        // '[' / ']' step back/forward through the result history (slice 10).
        KeyCode::Char('[') => {
            state.view = state.view.saturating_sub(1);
            return Vec::new();
        }
        KeyCode::Char(']') => {
            state.view = (state.view + 1).min(state.history.len().saturating_sub(1));
            return Vec::new();
        }
        _ => {}
    }
    let page = state.viewport_rows.max(1);
    if let Some(result) = state.shown_mut() {
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

/// Open the cell-detail overlay on the selected cell's Value (slice 08), if a
/// row and column are present. The Value is cloned so the overlay owns it and
/// the table's selection is untouched.
fn open_detail(state: &mut WorkbenchState) {
    let value = state.shown().and_then(|result| {
        result
            .rows
            .get(result.selected_row)
            .and_then(|row| row.fields().get(result.selected_col))
            .cloned()
    });
    if let Some(value) = value {
        state.detail = Some(value);
        state.detail_scroll = 0;
    }
}

/// Open the export prompt for the on-screen result (slice 09), if there is one.
fn open_export(state: &mut WorkbenchState) {
    if state.shown().is_some() {
        state.export = Some(ExportPrompt::default());
    } else {
        state.status.message = "no result to export".to_string();
    }
}

/// Keys while the export prompt is open: Tab cycles the format, typing edits the
/// destination path, Enter confirms (emitting the export effect), Esc cancels.
fn export_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    match key.code {
        KeyCode::Enter => return confirm_export(state),
        KeyCode::Esc => {
            state.export = None;
            return Vec::new();
        }
        _ => {}
    }
    if let Some(prompt) = state.export.as_mut() {
        match key.code {
            KeyCode::Tab => prompt.format = prompt.format.next(),
            KeyCode::Backspace => {
                prompt.path.pop();
            }
            KeyCode::Char(c) => prompt.path.push(c),
            _ => {}
        }
    }
    Vec::new()
}

/// Confirm an export: build the [`Effect::Export`] from the prompt and the
/// on-screen rows (a partial result after a cancel exports its partial rows). A
/// blank path keeps the prompt open with a hint.
fn confirm_export(state: &mut WorkbenchState) -> Vec<Effect> {
    let Some(prompt) = state.export.as_ref() else {
        return Vec::new();
    };
    if prompt.path.trim().is_empty() {
        state.status.message = "export: enter a destination path".to_string();
        return Vec::new();
    }
    let format = prompt.format;
    let path = PathBuf::from(prompt.path.trim());
    let Some(result) = state.shown() else {
        return Vec::new();
    };
    let header = result.header.clone();
    let rows = result.rows.clone();
    state.export = None;
    state.status.message = format!("exporting to {}…", path.display());
    vec![Effect::Export {
        format,
        path,
        header,
        rows,
    }]
}

/// Keys while the cell-detail overlay is open: scroll it, or dismiss it back to
/// the table (with the table selection intact, since it was never changed).
fn detail_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    match key.code {
        KeyCode::Up | KeyCode::PageUp => {
            state.detail_scroll = state.detail_scroll.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::PageDown => {
            state.detail_scroll = state.detail_scroll.saturating_add(1);
        }
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => state.detail = None,
        _ => {}
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
    fn shift_enter_inserts_a_newline_on_a_capable_terminal() {
        // On a keyboard-enhancement-capable terminal this chord arrives distinctly.
        let mut s = wb();
        type_str(&mut s, "ab");
        let shift_enter = Key {
            code: KeyCode::Enter,
            ctrl: false,
            alt: false,
            shift: true,
        };
        let effects = update(&mut s, Event::Key(shift_enter));
        type_str(&mut s, "cd");
        assert_eq!(s.editor.buffer(), "ab\ncd");
        assert!(effects.is_empty(), "a newline produces no effect");
    }

    #[test]
    fn ctrl_enter_inserts_a_newline_on_a_capable_terminal() {
        let mut s = wb();
        type_str(&mut s, "ab");
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Enter)));
        type_str(&mut s, "cd");
        assert_eq!(s.editor.buffer(), "ab\ncd");
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
        let result = s.shown().expect("a result");
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
        s.history.push(CurrentResult {
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
        assert_eq!(s.shown().unwrap().selected_row, 2);
        for _ in 0..10 {
            press(&mut s, KeyCode::Down);
        }
        assert_eq!(s.shown().unwrap().selected_row, 4, "clamped to last row");
        for _ in 0..10 {
            press(&mut s, KeyCode::Up);
        }
        assert_eq!(s.shown().unwrap().selected_row, 0, "clamped to first row");
    }

    #[test]
    fn home_and_end_jump_to_first_and_last_row() {
        let mut s = with_result(50, 10);
        press(&mut s, KeyCode::End);
        assert_eq!(s.shown().unwrap().selected_row, 49);
        press(&mut s, KeyCode::Home);
        assert_eq!(s.shown().unwrap().selected_row, 0);
    }

    #[test]
    fn column_navigation_clamps_to_the_header_width() {
        let mut s = with_result(3, 10);
        press(&mut s, KeyCode::Right);
        assert_eq!(s.shown().unwrap().selected_col, 1);
        press(&mut s, KeyCode::Right); // only two columns, so clamp at 1
        assert_eq!(s.shown().unwrap().selected_col, 1);
        press(&mut s, KeyCode::Left);
        assert_eq!(s.shown().unwrap().selected_col, 0);
    }

    #[test]
    fn the_scroll_window_follows_the_selection() {
        // Viewport of 3 rows over 20: paging down scrolls the window so the
        // selection stays visible (only the visible window is ever drawn).
        let mut s = with_result(20, 3);
        press(&mut s, KeyCode::PageDown);
        let r = s.shown().unwrap();
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
        let result = s.shown().unwrap();
        assert_eq!(result.rows.len(), 2, "held rows capped at the backstop");
        assert!(result.truncated, "truncation flagged");
    }

    // --- cancellation (slice 07) --------------------------------------------

    #[test]
    fn ctrl_c_cancels_a_running_query_and_keeps_partial_rows() {
        let mut s = wb();
        let id = submit_query(&mut s, "MATCH (n) RETURN n;");
        update(&mut s, Event::QueryStarted { id, header: vec!["n".to_string()] });
        update(&mut s, Event::RecordArrived { id, record: one_row() });
        update(&mut s, Event::RecordArrived { id, record: one_row() });
        let effects = update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('c'))));
        assert_eq!(effects, vec![Effect::Cancel { id }], "the edge is told to cancel");
        assert!(matches!(s.run, RunState::Idle), "session ready for the next query");
        let result = s.shown().unwrap();
        assert_eq!(result.rows.len(), 2, "streamed rows stay on screen");
        assert!(result.partial, "result labelled partial");
        assert!(s.status.message.contains("partial"), "status: {}", s.status.message);
        assert!(s.status.message.contains("2 rows"), "status names the count");
    }

    #[test]
    fn ctrl_c_drops_the_rest_of_a_multi_statement_batch() {
        let mut s = wb();
        let id = submit_query(&mut s, "RETURN 1; RETURN 2;");
        assert_eq!(s.pending.len(), 1);
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('c'))));
        assert!(s.pending.is_empty(), "the queued statement is dropped");
        let _ = id;
    }

    #[test]
    fn ctrl_c_when_idle_abandons_the_typed_buffer() {
        let mut s = wb();
        type_str(&mut s, "MATCH (n)");
        let effects = update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('c'))));
        assert!(effects.is_empty(), "nothing to cancel");
        assert_eq!(s.editor.buffer(), "", "the buffer is cleared");
    }

    #[test]
    fn the_spinner_advances_only_while_a_query_runs() {
        let mut s = wb();
        update(&mut s, Event::Tick);
        assert_eq!(s.spinner, 0, "idle ticks do not advance the spinner");
        submit_query(&mut s, "RETURN 1;");
        update(&mut s, Event::Tick);
        update(&mut s, Event::Tick);
        assert_eq!(s.spinner, 2, "running ticks advance the spinner");
    }

    // --- cell-detail overlay (slice 08) -------------------------------------

    /// A workbench whose results pane holds one selectable cell.
    fn with_one_cell(value: Value) -> WorkbenchState {
        let mut s = wb();
        s.focus = Focus::Results;
        s.history.push(CurrentResult {
            header: vec!["v".to_string()],
            rows: vec![Record::new(vec![value])],
            ..CurrentResult::default()
        });
        s
    }

    #[test]
    fn enter_expands_the_selected_cell_into_the_detail_overlay() {
        let mut s = with_one_cell(Value::String("Ada".into()));
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(s.detail, Some(Value::String("Ada".into())), "overlay holds the value");
    }

    #[test]
    fn esc_dismisses_the_overlay_and_keeps_the_table_selection() {
        let mut s = with_one_cell(Value::Integer(7));
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter))); // open
        s.shown_mut().unwrap().selected_row = 0;
        update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(s.detail.is_none(), "overlay closed");
        assert_eq!(s.shown().unwrap().selected_row, 0, "selection intact");
    }

    #[test]
    fn esc_while_the_overlay_is_open_does_not_quit() {
        let mut s = with_one_cell(Value::Integer(7));
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(effects.is_empty(), "Esc closes the overlay, it does not quit");
    }

    #[test]
    fn the_overlay_scrolls_with_the_arrow_keys() {
        let mut s = with_one_cell(Value::Integer(7));
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        update(&mut s, Event::Key(Key::plain(KeyCode::Down)));
        update(&mut s, Event::Key(Key::plain(KeyCode::Down)));
        assert_eq!(s.detail_scroll, 2);
        update(&mut s, Event::Key(Key::plain(KeyCode::Up)));
        assert_eq!(s.detail_scroll, 1);
    }

    // --- export (slice 09) --------------------------------------------------

    #[test]
    fn e_opens_the_export_prompt_and_enter_emits_the_export_effect() {
        use crate::workbench::effect::ExportFormat;
        let mut s = with_one_cell(Value::String("Ada".into()));
        update(&mut s, Event::Key(Key::char('e')));
        assert!(s.export.is_some(), "prompt opened");
        // Cycle the format once (csv -> jsonl) and type a path.
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        type_str(&mut s, "/tmp/out.jsonl");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::Export { format, path, rows, .. } => {
                assert_eq!(*format, ExportFormat::Jsonl);
                assert_eq!(path.to_str(), Some("/tmp/out.jsonl"));
                assert_eq!(rows.len(), 1, "exports the on-screen rows");
            }
            other => panic!("expected Export, got {other:?}"),
        }
        assert!(s.export.is_none(), "prompt closed on confirm");
    }

    #[test]
    fn export_with_a_blank_path_keeps_the_prompt_open() {
        let mut s = with_one_cell(Value::Integer(1));
        update(&mut s, Event::Key(Key::char('e')));
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty(), "no export with a blank path");
        assert!(s.export.is_some(), "prompt stays open");
    }

    #[test]
    fn esc_cancels_the_export_prompt_without_quitting() {
        let mut s = with_one_cell(Value::Integer(1));
        update(&mut s, Event::Key(Key::char('e')));
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(effects.is_empty(), "Esc cancels, does not quit");
        assert!(s.export.is_none());
    }

    #[test]
    fn export_finished_reports_the_outcome_in_the_status() {
        let mut s = wb();
        update(&mut s, Event::ExportFinished(Ok(PathBuf::from("/tmp/out.csv"))));
        assert!(s.status.message.contains("/tmp/out.csv"));
        update(&mut s, Event::ExportFinished(Err("disk full".to_string())));
        assert!(s.status.message.contains("disk full"));
    }

    // --- result history stack (slice 10) ------------------------------------

    fn complete(state: &mut WorkbenchState, id: u64, rows: usize) {
        update(state, Event::QueryStarted { id, header: vec!["n".to_string()] });
        for _ in 0..rows {
            update(state, Event::RecordArrived { id, record: one_row() });
        }
        update(
            state,
            Event::QueryCompleted {
                id,
                summary: Summary::default(),
                elapsed: Duration::from_millis(1),
            },
        );
    }

    #[test]
    fn a_multi_statement_submit_pushes_one_history_entry_per_statement() {
        let mut s = wb();
        let id0 = submit_query(&mut s, "RETURN 1; RETURN 2;");
        complete(&mut s, id0, 1); // first statement; advance starts the second (id 1)
        complete(&mut s, 1, 2); // second statement
        assert_eq!(s.history.len(), 2, "one entry per statement");
        assert_eq!(s.history[0].rows.len(), 1);
        assert_eq!(s.history[1].rows.len(), 2);
        assert_eq!(s.view, 1, "showing the latest");
    }

    #[test]
    fn back_and_forward_navigate_results_across_submits() {
        let mut s = wb();
        let id = submit_query(&mut s, "RETURN 1;");
        complete(&mut s, id, 1);
        let id = submit_query(&mut s, "RETURN 2;");
        complete(&mut s, id, 2);
        assert_eq!(s.history.len(), 2);
        s.focus = Focus::Results;
        press(&mut s, KeyCode::Char('['));
        assert_eq!(s.view, 0);
        press(&mut s, KeyCode::Char('['));
        assert_eq!(s.view, 0, "clamped at the oldest");
        press(&mut s, KeyCode::Char(']'));
        assert_eq!(s.view, 1);
        press(&mut s, KeyCode::Char(']'));
        assert_eq!(s.view, 1, "clamped at the newest");
    }

    #[test]
    fn a_revisited_result_keeps_its_own_rows() {
        let mut s = wb();
        let id = submit_query(&mut s, "RETURN 1;");
        complete(&mut s, id, 3);
        let id = submit_query(&mut s, "RETURN 2;");
        complete(&mut s, id, 1);
        s.focus = Focus::Results;
        press(&mut s, KeyCode::Char('[')); // back to the first result
        assert_eq!(
            s.shown().unwrap().rows.len(),
            3,
            "the older result's rows are intact"
        );
    }

    // --- completion popup (slice 11) ----------------------------------------

    #[test]
    fn tab_opens_completion_for_the_word_under_the_cursor() {
        let mut s = wb();
        type_str(&mut s, "MAT");
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        let completion = s.completion.as_ref().expect("popup open");
        assert!(
            completion.candidates.contains(&"MATCH".to_string()),
            "candidates: {:?}",
            completion.candidates
        );
        assert_eq!(completion.prefix_len, 3);
    }

    #[test]
    fn enter_inserts_the_selected_candidate_replacing_the_prefix() {
        let mut s = wb();
        type_str(&mut s, "RETUR");
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab))); // open (RETURN matches)
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter))); // insert
        assert_eq!(s.editor.buffer(), "RETURN");
        assert!(s.completion.is_none(), "popup closed after insert");
    }

    #[test]
    fn down_cycles_the_selection_and_wraps() {
        let mut s = wb();
        type_str(&mut s, "RE"); // several keyword matches
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        let count = s.completion.as_ref().unwrap().candidates.len();
        assert!(count > 1, "needs multiple candidates to cycle");
        update(&mut s, Event::Key(Key::plain(KeyCode::Down)));
        assert_eq!(s.completion.as_ref().unwrap().selected, 1);
        update(&mut s, Event::Key(Key::plain(KeyCode::Up)));
        update(&mut s, Event::Key(Key::plain(KeyCode::Up)));
        assert_eq!(s.completion.as_ref().unwrap().selected, count - 1, "wraps past the top");
    }

    #[test]
    fn esc_dismisses_the_completion_popup() {
        let mut s = wb();
        type_str(&mut s, "MAT");
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(s.completion.is_none());
    }

    #[test]
    fn tab_with_no_completable_prefix_moves_focus_to_results() {
        // An empty prefix offers nothing (the static completer declines it), so
        // Tab falls back to cycling focus.
        let mut s = wb();
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert!(s.completion.is_none());
        assert_eq!(s.focus, Focus::Results);
    }

    // --- live schema completion source (slice 12) ---------------------------

    #[test]
    fn a_loaded_schema_contributes_completion_candidates() {
        // Names chosen not to collide with any static keyword/function prefix.
        let mut s = wb();
        let schema = Schema {
            labels: vec!["Wombat".to_string()],
            rel_types: vec!["GNAWS_AT".to_string()],
            property_keys: vec!["furriness".to_string()],
        };
        update(&mut s, Event::SchemaLoaded(Some(schema)));
        // The label is offered alongside the static keyword/function vocabulary.
        assert!(s.completer.candidates("Wom").contains(&"Wombat".to_string()));
        assert!(s.completer.candidates("RET").contains(&"RETURN".to_string()));
        assert!(s.schema.is_some(), "schema kept for the sidebar");
    }

    #[test]
    fn no_schema_degrades_silently_to_static_only_completion() {
        let mut s = wb();
        update(&mut s, Event::SchemaLoaded(None));
        assert!(s.schema.is_none());
        // Static vocabulary still completes; no schema names are present.
        assert!(s.completer.candidates("RET").contains(&"RETURN".to_string()));
        assert!(s.completer.candidates("Wom").is_empty());
    }

    #[test]
    fn a_refetched_schema_replaces_rather_than_stacks() {
        let mut s = wb();
        update(
            &mut s,
            Event::SchemaLoaded(Some(Schema {
                labels: vec!["Wombat".to_string()],
                ..Schema::default()
            })),
        );
        update(
            &mut s,
            Event::SchemaLoaded(Some(Schema {
                labels: vec!["Zonk".to_string()],
                ..Schema::default()
            })),
        );
        // The stale label is gone, the fresh one present (replaced, not stacked).
        assert!(s.completer.candidates("Wom").is_empty());
        assert!(s.completer.candidates("Zon").contains(&"Zonk".to_string()));
    }

    #[test]
    fn ctrl_r_requests_a_schema_refresh() {
        let mut s = wb();
        let effects = update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('r'))));
        assert_eq!(effects, vec![Effect::FetchSchema]);
    }

    // --- schema sidebar (slice 13) ------------------------------------------

    #[test]
    fn ctrl_b_toggles_the_schema_sidebar_when_a_schema_is_loaded() {
        let mut s = wb();
        update(
            &mut s,
            Event::SchemaLoaded(Some(Schema {
                labels: vec!["Person".to_string()],
                ..Schema::default()
            })),
        );
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('b'))));
        assert_eq!(s.drawer, Some(DrawerKind::Schema), "opened");
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('b'))));
        assert_eq!(s.drawer, None, "toggled closed");
    }

    #[test]
    fn the_sidebar_is_unavailable_without_a_schema() {
        let mut s = wb();
        update(&mut s, Event::SchemaLoaded(None)); // feature off
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('b'))));
        assert_eq!(s.drawer, None, "drawer does not open when there is no schema");
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
        assert_eq!(s.shown().expect("result").rows.len(), 0);
    }
}
