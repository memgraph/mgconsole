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
use crate::theme::{Chord, ChordKey, Gesture};

use super::effect::{Effect, TxOp};
use super::event::{Event, Key, KeyCode};
use super::plan::{is_plan_query, Plan};
use super::schema::{Schema, SchemaSource};
use super::state::{
    Completion, CurrentResult, DrawerKind, ExportPrompt, Focus, RunState, WatchState,
    WorkbenchState,
};

/// Apply one event to the state, returning the effects to perform.
// Events are consumed by value: slice 02's lifecycle events carry owned data
// (a Record, a Summary, an Error) the reducer takes ownership of. Today's
// Key/Resize payloads are Copy, so the lint is transitional to this slice.
// One arm per event in the workbench's vocabulary; the match grows with each new
// lifecycle/command event (issues 02–12). Splitting it would scatter the small
// per-event state edits for no clarity gain.
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
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
        Event::HistoryLoaded(entries) => {
            state.history_entries = entries;
            Vec::new()
        }
        Event::TransactionApplied { state: tx, message } => {
            state.tx = tx;
            if let Some(message) = message {
                state.status.message = message;
            }
            Vec::new()
        }
        Event::Connected(result) => {
            match result {
                Ok(connected) => {
                    state.endpoint = connected.endpoint;
                    state.profile = connected.profile;
                    state.read_only = connected.read_only;
                    state.tx = mgconsole_core::TransactionState::Auto;
                    state.status.message = format!("connected to {}", connected.label);
                }
                // A failed connect leaves the prior Session (and its display) intact.
                Err(message) => state.status.message = format!("error: {message}"),
            }
            Vec::new()
        }
        Event::DatabaseChanged(result) => {
            match result {
                Ok(database) => {
                    state.status.message = format!("using database {database}");
                    state.database = Some(database);
                }
                // A failed switch leaves the current Database active.
                Err(message) => state.status.message = format!("error: {message}"),
            }
            Vec::new()
        }
        Event::Redirected { id, result } => {
            if !is_current(state, id) {
                return Vec::new();
            }
            state.status.message = match result {
                Ok((path, rows)) => format!("wrote {rows} row(s) to {}", path.display()),
                Err(message) => format!("error: {message}"),
            };
            // Like a completed query: run the next pending statement or fall idle.
            advance(state)
        }
        Event::SourceLoaded(result) => match result {
            Ok(content) => {
                let statements = split_statements(&content);
                if statements.is_empty() {
                    return Vec::new();
                }
                // Run as a stop-on-error batch (issue 10).
                state.source_halt = true;
                let mut statements = statements.into_iter();
                let first = statements.next().expect("non-empty checked");
                state.pending = statements.collect();
                start_query(state, first)
            }
            Err(message) => {
                state.status.message = format!("error: {message}");
                Vec::new()
            }
        },
        Event::ParamEvaluated { name, value } => {
            match value {
                Ok(value) => {
                    state.params.insert(name.clone(), value);
                    state.status.message = format!("set ${name}");
                }
                // The Session survives a bad expression (ADR 0005); just report it.
                Err(message) => state.status.message = format!("error: {message}"),
            }
            Vec::new()
        }
        Event::Tick => on_tick(state),
    }
}

/// The render-loop tick period (ms). `:watch` counts ticks against it (issue 11).
pub const TICK_MS: u64 = 120;

/// How many ticks one `:watch` interval spans (at least one).
fn period_ticks(interval: Duration) -> u32 {
    let ticks = u64::try_from(interval.as_millis()).unwrap_or(u64::MAX) / TICK_MS;
    u32::try_from(ticks.max(1)).unwrap_or(u32::MAX)
}

/// Advance the spinner and the `:watch` timer on each render-loop tick. When a
/// watch interval elapses (and the Session is idle), re-run its query, replacing
/// the previous snapshot rather than accumulating (issue 11).
fn on_tick(state: &mut WorkbenchState) -> Vec<Effect> {
    if matches!(state.run, RunState::Running { .. }) {
        state.spinner = state.spinner.wrapping_add(1);
    }
    let fire = match state.watch.as_mut() {
        Some(watch) if matches!(state.run, RunState::Idle) => {
            if watch.remaining <= 1 {
                watch.remaining = watch.period_ticks;
                Some(watch.query.clone())
            } else {
                watch.remaining -= 1;
                None
            }
        }
        _ => None,
    };
    if let Some(query) = fire {
        // Replace the previous snapshot so a long watch does not accumulate.
        state.history.pop();
        state.view = state.history.len().saturating_sub(1);
        start_query(state, query)
    } else {
        Vec::new()
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
    let statement = state.running_statement.clone().unwrap_or_default();
    state.history.push(CurrentResult::new(statement, header));
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
    // Surface a notification count in the status line (full detail in the
    // summary drawer, slice 18); the drawer reads the stored Summary.
    let note = match summary.notifications.len() {
        0 => String::new(),
        1 => " · 1 notification".to_string(),
        n => format!(" · {n} notifications"),
    };
    state.status.message = format!("{}{note}", format_summary(rows, elapsed));
    if let Some(result) = state.live_mut() {
        result.summary = Some(summary);
        // An EXPLAIN/PROFILE result renders as an operator tree, not a table.
        if is_plan_query(&result.statement) {
            result.plan = Some(Plan::parse(&result.rows));
        }
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
    // A `:source` batch stops on the first error (issue 10): drop the rest.
    if state.source_halt {
        state.source_halt = false;
        state.pending.clear();
        state.run = RunState::Idle;
        return Vec::new();
    }
    advance(state)
}

/// Run the next pending statement of a multi-statement submit, or fall idle when
/// the batch is done.
fn advance(state: &mut WorkbenchState) -> Vec<Effect> {
    if let Some(next) = state.pending.pop_front() {
        start_query(state, next)
    } else {
        state.run = RunState::Idle;
        // The `:source` batch (issue 10) finished cleanly; leave stop-on-error mode.
        state.source_halt = false;
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
    state.running_statement = Some(query.clone());
    // Remember the last query so `:watch` with no query can reuse it (issue 11).
    state.last_query = Some(query.clone());
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
    // While `:watch` is active, any key stops it (issue 11) and is consumed; a
    // running watch re-run is cancelled so the Session is freed.
    if state.watch.is_some() {
        state.watch = None;
        state.status.message = "watch stopped".to_string();
        if matches!(state.run, RunState::Running { .. }) {
            return interrupt(state);
        }
        return Vec::new();
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
    // The rebindable gestures (issue 14): a key press is matched against the
    // resolved `[keys]` bindings (defaults reproduce today's chords). The buffer
    // gestures are bound here too but acted on in issue 18.
    if let Some(chord) = chord_of(key) {
        if let Some(gesture) = state.keys.gesture_for(chord) {
            return handle_gesture(state, gesture);
        }
    }
    match state.focus {
        Focus::Editor => editor_key(state, key),
        Focus::Results => results_key(state, key),
    }
}

/// Map a workbench [`Key`] onto a neutral [`Chord`] for keybinding lookup (issue
/// 14), or `None` for a key the binding layer never matches (a bare printable
/// character, [`KeyCode::Other`]). Bare characters are excluded so ordinary typing
/// is never swallowed by a gesture binding.
fn chord_of(key: Key) -> Option<Chord> {
    let chord_key = match key.code {
        KeyCode::Char(c) if key.ctrl || key.alt => ChordKey::Char(c.to_ascii_lowercase()),
        KeyCode::Char(_) | KeyCode::Other => return None,
        KeyCode::Enter => ChordKey::Enter,
        KeyCode::Esc => ChordKey::Esc,
        KeyCode::Tab => ChordKey::Tab,
        KeyCode::Backspace => ChordKey::Backspace,
        KeyCode::Delete => ChordKey::Delete,
        KeyCode::Up => ChordKey::Up,
        KeyCode::Down => ChordKey::Down,
        KeyCode::Left => ChordKey::Left,
        KeyCode::Right => ChordKey::Right,
        KeyCode::Home => ChordKey::Home,
        KeyCode::End => ChordKey::End,
        KeyCode::PageUp => ChordKey::PageUp,
        KeyCode::PageDown => ChordKey::PageDown,
    };
    Some(Chord {
        ctrl: key.ctrl,
        alt: key.alt,
        shift: key.shift,
        key: chord_key,
    })
}

/// Run a rebindable gesture (issue 14). The drawer/schema toggles act here; the
/// Buffer gestures are matched but not yet acted on (issue 18 fills them in).
fn handle_gesture(state: &mut WorkbenchState, gesture: Gesture) -> Vec<Effect> {
    match gesture {
        Gesture::RefreshSchema => {
            state.status.message = "refreshing schema…".to_string();
            vec![Effect::FetchSchema]
        }
        Gesture::ToggleSchema => {
            toggle_schema_sidebar(state);
            Vec::new()
        }
        Gesture::ToggleParams => {
            state.drawer = toggle_drawer(state.drawer, DrawerKind::Params);
            Vec::new()
        }
        Gesture::ToggleSummary => {
            state.drawer = toggle_drawer(state.drawer, DrawerKind::Summary);
            Vec::new()
        }
        // Buffer (tab) gestures arrive in issue 18; recognised now so their chords
        // are reserved and rebindable, a no-op until then.
        Gesture::NewBuffer
        | Gesture::CloseBuffer
        | Gesture::NextBuffer
        | Gesture::PrevBuffer => Vec::new(),
    }
}

/// Toggle `drawer` to `kind`, or closed if it is already showing `kind`.
fn toggle_drawer(drawer: Option<DrawerKind>, kind: DrawerKind) -> Option<DrawerKind> {
    if drawer == Some(kind) {
        None
    } else {
        Some(kind)
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
        // Ctrl+Up / Ctrl+Down recall older / newer history into the editor
        // (slice 17); plain Up/Down move the cursor (ordinary editing, below).
        Key {
            code: KeyCode::Up,
            ctrl: true,
            ..
        } => {
            recall_older(state);
            Vec::new()
        }
        Key {
            code: KeyCode::Down,
            ctrl: true,
            ..
        } => {
            recall_newer(state);
            Vec::new()
        }
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
            // For a plan result, Enter collapses/expands the selected operator;
            // for a table, it expands the selected cell (slice 08/14).
            let toggled = state
                .shown_mut()
                .and_then(|result| result.plan.as_mut())
                .map(Plan::toggle)
                .is_some();
            if !toggled {
                open_detail(state);
            }
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
    // Plan results navigate as an operator tree (slice 14); tables navigate rows.
    if let Some(plan) = state.shown_mut().and_then(|result| result.plan.as_mut()) {
        match key.code {
            KeyCode::Up => plan.move_selection(-1),
            KeyCode::Down => plan.move_selection(1),
            KeyCode::Left => plan.collapse(),
            KeyCode::Right => plan.expand(),
            _ => {}
        }
        return Vec::new();
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
            KeyCode::Tab => prompt.format = super::effect::next_export_format(prompt.format),
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
    // A `:`-meta command is handled here (reusing the REPL's parser), not run as
    // a query (slice 16 brings the `:param` family into the workbench).
    if let Some(meta) = meta_command(buffer.trim()) {
        return handle_meta(state, meta);
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
    // A `:o` redirect (issue 12) sends the first statement to a file instead of
    // the result pane (one-shot); the rest run normally after.
    let mut effects = if let Some((format, path)) = state.redirect.take() {
        start_query_to_file(state, first, format, path)
    } else {
        start_query(state, first)
    };
    // The whole submission is one recallable history entry (slice 17).
    record_history(state, &buffer, &mut effects);
    effects
}

/// Begin a `:o`-redirected query (issue 12): like [`start_query`] but the result
/// streams to a file instead of the pane, emitting [`Effect::RunQueryToFile`].
fn start_query_to_file(
    state: &mut WorkbenchState,
    query: String,
    format: crate::OutputFormat,
    path: std::path::PathBuf,
) -> Vec<Effect> {
    let id = state.next_id;
    state.next_id += 1;
    state.run = RunState::Running { id };
    state.spinner = 0;
    state.status.message = format!("running… → {} ({format})", path.display());
    state.running_statement = Some(query.clone());
    state.last_query = Some(query.clone());
    vec![Effect::RunQueryToFile {
        id,
        query,
        params: state.params.clone(),
        format,
        path,
    }]
}

/// Recall an older history entry into the editor (Ctrl+Up). On the first step the
/// live buffer is saved so stepping back past the newest restores it.
fn recall_older(state: &mut WorkbenchState) {
    if state.history_entries.is_empty() {
        return;
    }
    let index = match state.recall_index {
        None => {
            state.recall_saved = Some(state.editor.buffer());
            state.history_entries.len() - 1
        }
        Some(i) => i.saturating_sub(1),
    };
    state.recall_index = Some(index);
    let text = state.history_entries[index].clone();
    state.editor.set_text(&text);
}

/// Recall a newer history entry (Ctrl+Down); stepping past the newest restores
/// the saved live buffer.
fn recall_newer(state: &mut WorkbenchState) {
    let Some(index) = state.recall_index else {
        return;
    };
    if index + 1 < state.history_entries.len() {
        state.recall_index = Some(index + 1);
        let text = state.history_entries[index + 1].clone();
        state.editor.set_text(&text);
    } else {
        state.recall_index = None;
        let saved = state.recall_saved.take().unwrap_or_default();
        state.editor.set_text(&saved);
    }
}

/// Record a submitted query in history (slice 17): append it in-memory (skipping
/// a consecutive duplicate) and emit the persist effect; reset recall.
fn record_history(state: &mut WorkbenchState, submission: &str, effects: &mut Vec<Effect>) {
    let entry = submission.trim().to_string();
    if entry.is_empty() {
        return;
    }
    state.recall_index = None;
    state.recall_saved = None;
    if state.history_entries.last() != Some(&entry) {
        state.history_entries.push(entry.clone());
    }
    effects.push(Effect::AppendHistory(entry));
}

/// Handle a submitted `:`-meta command, reusing the REPL's `MetaCommand`. The
/// `:param` family (slice 16) reuses the REPL's store and server-side evaluation;
/// the editor is cleared since a command is consumed (unlike a query, which is
/// kept for re-run).
// One arm per command in the shared vocabulary; the match grows with each new
// `:`-command (issues 04–13). Splitting it would scatter the small per-arm state
// edits for no clarity gain — it is the Workbench analogue of the REPL's
// `dispatch_meta`.
#[allow(clippy::too_many_lines)]
fn handle_meta(state: &mut WorkbenchState, meta: MetaCommand) -> Vec<Effect> {
    match meta {
        MetaCommand::Quit => vec![Effect::Quit],
        MetaCommand::SetParam { name, expr } => {
            // Evaluation runs a query; refuse while one is in flight (ADR 0005).
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
                return Vec::new();
            }
            state.editor.clear();
            vec![Effect::EvaluateParam {
                name,
                expr,
                params: state.params.clone(),
            }]
        }
        MetaCommand::ListParams => {
            state.drawer = Some(DrawerKind::Params);
            state.editor.clear();
            Vec::new()
        }
        MetaCommand::ClearParams => {
            state.params.clear();
            state.status.message = "parameters cleared".to_string();
            state.editor.clear();
            Vec::new()
        }
        // `:set` shares the REPL's Settings spine (issue 01): list every setting,
        // or change one and report it — kept distinct from the `:param` store.
        // `readonly` (issue 04) is special: a Session guard, turned off only at
        // connect time.
        MetaCommand::ListSettings => {
            state.status.message = format!(
                "{} · readonly = {}",
                state.settings.list().replace('\n', " · "),
                crate::repl::on_off(state.read_only)
            );
            state.editor.clear();
            Vec::new()
        }
        MetaCommand::SetSetting { name, value } if name == "readonly" => {
            state.editor.clear();
            match crate::repl::parse_on_off(&value) {
                Ok(true) => {
                    state.read_only = true;
                    state.status.message = "readonly = on".to_string();
                    return vec![Effect::SetReadOnly(true)];
                }
                Ok(false) => {
                    state.status.message =
                        "error: read-only can only be turned off at connect time, not at runtime"
                            .to_string();
                }
                Err(message) => state.status.message = format!("error: {message}"),
            }
            Vec::new()
        }
        MetaCommand::SetSetting { name, value } => {
            match state.settings.set(&name, &value) {
                Ok(()) => {
                    // `:set theme` repaints: re-resolve the palette over the newly
                    // named built-in base with the config `[theme]` overrides still
                    // applied (issue 14).
                    if name == "theme" {
                        let (palette, _warnings) = crate::theme::resolve_palette(
                            &state.settings.theme,
                            &state.config.theme_overrides,
                        );
                        state.palette = palette;
                    }
                    state.status.message =
                        format!("{name} = {}", state.settings.get(&name).unwrap_or(value));
                }
                // The session survives a bad name/value (ADR 0005); just report it.
                Err(message) => state.status.message = format!("error: {message}"),
            }
            state.editor.clear();
            Vec::new()
        }
        // Explicit transactions (issue 05): the reducer requests the effect; the
        // edge applies it to the Session and reports the outcome via an event.
        // Evaluation runs on the Session, so refuse while a query is in flight.
        MetaCommand::Begin | MetaCommand::Commit | MetaCommand::Rollback => {
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
                return Vec::new();
            }
            state.editor.clear();
            vec![Effect::Transaction(match meta {
                MetaCommand::Begin => TxOp::Begin,
                MetaCommand::Commit => TxOp::Commit,
                _ => TxOp::Rollback,
            })]
        }
        // `:connect` swaps the whole Session (issue 07). Refuse mid-query; warn
        // that an open transaction is aborted by the swap (ADR 0011).
        MetaCommand::Connect(target) => {
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
                return Vec::new();
            }
            state.editor.clear();
            if state.tx != mgconsole_core::TransactionState::Auto {
                state.status.message = "note: the open transaction is aborted by :connect".to_string();
            }
            vec![Effect::Connect(target)]
        }
        // `:use` switches the active Database (issue 08); refuse mid-query.
        MetaCommand::Use(database) => {
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
                return Vec::new();
            }
            state.editor.clear();
            vec![Effect::UseDatabase(database)]
        }
        // `:o` arms the next submitted query to stream to a file (issue 12);
        // one-shot. An unknown format/extension is reported.
        MetaCommand::Redirect(args) => {
            state.editor.clear();
            match crate::repl::parse_redirect(&args) {
                Ok((format, path)) => {
                    state.status.message =
                        format!("next query → {} ({format})", path.display());
                    state.redirect = Some((format, path));
                }
                Err(message) => state.status.message = format!("error: {message}"),
            }
            Vec::new()
        }
        // `:source` reads a file (issue 10) then runs its statements as a
        // stop-on-error batch. The file read is an effect; the run happens when
        // its contents arrive. (Interleaved meta-commands in a sourced file are a
        // REPL feature; the Workbench sources Cypher statements.)
        MetaCommand::Source(path) => {
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
                return Vec::new();
            }
            state.editor.clear();
            vec![Effect::Source(PathBuf::from(path))]
        }
        // `:watch` re-runs a query on a timer (issue 11), refused while a
        // transaction is open. The first run starts immediately; the tick timer
        // drives the rest, each replacing the previous snapshot. Any key stops it.
        MetaCommand::Watch(args) => {
            if state.tx != mgconsole_core::TransactionState::Auto {
                state.status.message =
                    "error: :watch is refused while a transaction is open".to_string();
                return Vec::new();
            }
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
                return Vec::new();
            }
            state.editor.clear();
            match crate::repl::parse_watch(&args, state.last_query.as_deref()) {
                Ok(spec) => {
                    let period = period_ticks(spec.interval);
                    state.watch = Some(WatchState {
                        query: spec.query.clone(),
                        period_ticks: period,
                        remaining: period,
                    });
                    state.status.message = format!(
                        "watching every {:.1}s — press any key to stop",
                        spec.interval.as_secs_f64()
                    );
                    start_query(state, spec.query)
                }
                Err(message) => {
                    state.status.message = format!("error: {message}");
                    Vec::new()
                }
            }
        }
        // `:sysinfo` runs the server-status queries as a normal batch (issue 09):
        // each result lands in the result pane, rendered like any other.
        MetaCommand::Sysinfo => {
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
                return Vec::new();
            }
            state.editor.clear();
            let mut statements = crate::repl::SYSINFO_QUERIES
                .iter()
                .map(|q| (*q).to_string());
            let Some(first) = statements.next() else {
                return Vec::new();
            };
            state.pending = statements.collect();
            start_query(state, first)
        }
        // Named queries (issue 13): the same verbs as the REPL, over the same
        // store. `:save`/`:forget` mutate the in-memory store in this pure reducer
        // and request a persist effect; `:load` recalls into the editor (never
        // auto-runs); `:saved` summarises into the status bar.
        MetaCommand::Save { name, query } => {
            state.editor.clear();
            match query.or_else(|| state.last_query.clone()) {
                Some(text) => {
                    state.queries.set(name.clone(), text);
                    state.status.message = format!("saved '{name}'");
                    return vec![Effect::PersistQueries];
                }
                None => {
                    state.status.message =
                        format!("error: no query to save; give one: ':save {name} <query>'");
                }
            }
            Vec::new()
        }
        MetaCommand::Saved => {
            state.status.message = state.queries.summary();
            state.editor.clear();
            Vec::new()
        }
        // `:load` recalls the template into the editor for review/edit — it never
        // auto-runs (issue 13). The editor keeps it so the user can submit it.
        MetaCommand::Load(name) => {
            match state.queries.get(&name) {
                Some(text) => {
                    let text = text.to_string();
                    state.editor.set_text(&text);
                    state.status.message = format!("loaded '{name}' — edit and submit to run");
                }
                None => state.status.message = format!("error: no saved query named '{name}'"),
            }
            Vec::new()
        }
        MetaCommand::Forget(name) => {
            state.editor.clear();
            if state.queries.remove(&name) {
                state.status.message = format!("forgot '{name}'");
                return vec![Effect::PersistQueries];
            }
            state.status.message = format!("error: no saved query named '{name}'");
            Vec::new()
        }
        MetaCommand::Invalid(message) => {
            state.status.message = format!("error: {message}");
            Vec::new()
        }
        MetaCommand::Unknown(command) => {
            state.status.message = format!("unknown command '{command}'");
            Vec::new()
        }
        // The help/docs text lands in slice 19; acknowledge for now.
        MetaCommand::Help | MetaCommand::Docs => {
            state.status.message = "help/docs arrive in slice 19".to_string();
            Vec::new()
        }
    }
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
            effects[0],
            Effect::RunQuery {
                id: 0,
                query: "RETURN 1".to_string(),
                params: BTreeMap::new(),
            }
        );
        // The submission is also recorded in history (slice 17).
        assert!(matches!(effects.get(1), Some(Effect::AppendHistory(_))));
        assert!(matches!(s.run, RunState::Running { id: 0 }));
        assert_eq!(s.editor.buffer(), "RETURN 1;", "the buffer is kept for re-run");
    }

    #[test]
    fn a_buffer_with_no_semicolon_runs_as_a_single_query() {
        let mut s = wb();
        type_str(&mut s, "RETURN 1");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(
            effects[0],
            Effect::RunQuery {
                id: 0,
                query: "RETURN 1".to_string(),
                params: BTreeMap::new(),
            }
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
        use crate::OutputFormat;
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
                assert_eq!(*format, OutputFormat::Jsonl);
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

    // --- EXPLAIN/PROFILE plan tree (slice 14) -------------------------------

    fn plan_row(text: &str) -> Record {
        Record::new(vec![Value::String(text.to_string())])
    }

    #[test]
    fn an_explain_query_renders_as_a_plan_not_a_table() {
        let mut s = wb();
        let id = submit_query(&mut s, "EXPLAIN MATCH (n) RETURN n;");
        update(&mut s, Event::QueryStarted { id, header: vec!["QUERY PLAN".to_string()] });
        update(&mut s, Event::RecordArrived { id, record: plan_row(" * Produce {n}") });
        update(&mut s, Event::RecordArrived { id, record: plan_row(" * ScanAll (n)") });
        update(
            &mut s,
            Event::QueryCompleted {
                id,
                summary: Summary::default(),
                elapsed: Duration::from_millis(1),
            },
        );
        let plan = s.shown().unwrap().plan.as_ref().expect("plan built");
        assert_eq!(plan.lines.len(), 2);
        assert_eq!(plan.lines[0].operator, "Produce {n}");
    }

    #[test]
    fn an_ordinary_query_stays_a_table() {
        let mut s = wb();
        let id = submit_query(&mut s, "MATCH (n) RETURN n;");
        complete(&mut s, id, 2);
        assert!(s.shown().unwrap().plan.is_none(), "no plan for an ordinary query");
    }

    #[test]
    fn plan_navigation_and_collapse_work_on_the_tree() {
        let mut s = wb();
        let id = submit_query(&mut s, "PROFILE MATCH (n) RETURN n;");
        update(&mut s, Event::QueryStarted { id, header: vec!["OPERATOR".to_string()] });
        update(&mut s, Event::RecordArrived { id, record: plan_row("* Produce {n}") });
        update(&mut s, Event::RecordArrived { id, record: plan_row("  * ScanAll (n)") });
        update(
            &mut s,
            Event::QueryCompleted { id, summary: Summary::default(), elapsed: Duration::from_millis(1) },
        );
        s.focus = Focus::Results;
        // Down moves the plan selection (not a table row).
        press(&mut s, KeyCode::Down);
        assert_eq!(s.shown().unwrap().plan.as_ref().unwrap().selected, 1);
        // Enter on the root (a node with a child) collapses the subtree.
        press(&mut s, KeyCode::Up);
        press(&mut s, KeyCode::Enter);
        assert!(s.shown().unwrap().plan.as_ref().unwrap().lines[0].collapsed);
    }

    // --- workbench parameters (slice 16) ------------------------------------

    #[test]
    fn set_param_evaluates_server_side_with_current_params_in_scope() {
        let mut s = wb();
        s.params.insert("x".to_string(), Value::Integer(1));
        type_str(&mut s, ":param y $x + 1");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(
            effects,
            vec![Effect::EvaluateParam {
                name: "y".to_string(),
                expr: "$x + 1".to_string(),
                params: BTreeMap::from([("x".to_string(), Value::Integer(1))]),
            }]
        );
        assert_eq!(s.editor.buffer(), "", "the command is consumed");
    }

    #[test]
    fn an_evaluated_param_is_stored_and_bound_to_the_next_query() {
        let mut s = wb();
        update(
            &mut s,
            Event::ParamEvaluated { name: "age".to_string(), value: Ok(Value::Integer(42)) },
        );
        assert_eq!(s.params.get("age"), Some(&Value::Integer(42)));
        // The stored param is bound to the next query (slice 02 binding).
        type_str(&mut s, "RETURN $age;");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        match &effects[0] {
            Effect::RunQuery { params, .. } => {
                assert_eq!(params.get("age"), Some(&Value::Integer(42)));
            }
            other => panic!("expected RunQuery, got {other:?}"),
        }
    }

    #[test]
    fn a_bad_param_expression_is_reported_without_losing_the_session() {
        let mut s = wb();
        update(
            &mut s,
            Event::ParamEvaluated { name: "x".to_string(), value: Err("syntax error".to_string()) },
        );
        assert!(s.status.message.contains("syntax error"));
        assert!(s.params.is_empty(), "nothing stored on failure");
        // The next query still runs.
        let id = submit_query(&mut s, "RETURN 1;");
        assert_eq!(id, 0);
    }

    #[test]
    fn params_commands_list_and_clear_the_store() {
        let mut s = wb();
        s.params.insert("n".to_string(), Value::Integer(7));
        // `:params` opens the parameters drawer.
        type_str(&mut s, ":params");
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(s.drawer, Some(DrawerKind::Params));
        // `:params clear` empties the store.
        type_str(&mut s, ":params clear");
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(s.params.is_empty());
    }

    #[test]
    fn ctrl_p_toggles_the_parameters_drawer() {
        let mut s = wb();
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('p'))));
        assert_eq!(s.drawer, Some(DrawerKind::Params));
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('p'))));
        assert_eq!(s.drawer, None);
    }

    // --- :set settings spine (issue 01) -------------------------------------

    #[test]
    fn set_display_changes_the_setting_and_clears_the_editor() {
        use mgconsole_core::DisplayMode;
        let mut s = wb();
        type_str(&mut s, ":set display vertical");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty(), "a setting change runs no query");
        assert_eq!(s.settings.display, DisplayMode::Vertical);
        assert_eq!(s.editor.buffer(), "", "the command is consumed");
        assert!(s.status.message.contains("display = vertical"));
    }

    #[test]
    fn bare_set_lists_the_settings_in_the_status() {
        let mut s = wb();
        type_str(&mut s, ":set");
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(s.status.message.contains("display = auto"), "status: {}", s.status.message);
    }

    #[test]
    fn an_invalid_setting_is_reported_without_losing_the_session() {
        use mgconsole_core::DisplayMode;
        let mut s = wb();
        type_str(&mut s, ":set display grid");
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(s.status.message.contains("error"), "status: {}", s.status.message);
        assert_eq!(s.settings.display, DisplayMode::Auto, "unchanged on error");
        // The session survives: a following query still runs.
        let id = submit_query(&mut s, "RETURN 1;");
        assert_eq!(id, 0);
    }

    #[test]
    fn set_does_not_touch_the_param_store() {
        let mut s = wb();
        type_str(&mut s, ":set display vertical");
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(s.params.is_empty(), ":set must not populate the :param store");
    }

    #[test]
    fn set_readonly_on_marks_state_and_emits_the_effect() {
        let mut s = wb();
        type_str(&mut s, ":set readonly on");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(s.read_only, "state marked read-only");
        assert_eq!(effects, vec![Effect::SetReadOnly(true)], "session told to apply it");
        assert_eq!(s.editor.buffer(), "", "command consumed");
    }

    #[test]
    fn set_readonly_off_at_runtime_is_refused() {
        let mut s = wb();
        s.read_only = true;
        type_str(&mut s, ":set readonly off");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(s.read_only, "still read-only — runtime off is refused");
        assert!(effects.is_empty(), "no effect on a refused change");
        assert!(s.status.message.contains("connect time"), "status: {}", s.status.message);
    }

    #[test]
    fn bare_set_lists_readonly_in_the_status() {
        let mut s = wb();
        type_str(&mut s, ":set");
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(s.status.message.contains("readonly = off"), "status: {}", s.status.message);
    }

    // --- explicit transactions (issue 05) -----------------------------------

    #[test]
    fn begin_emits_a_transaction_effect_and_clears_the_editor() {
        let mut s = wb();
        type_str(&mut s, ":begin");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(effects, vec![Effect::Transaction(TxOp::Begin)]);
        assert_eq!(s.editor.buffer(), "", "command consumed");
    }

    #[test]
    fn commit_and_rollback_emit_their_effects() {
        let mut s = wb();
        type_str(&mut s, ":commit");
        assert_eq!(
            update(&mut s, Event::Key(Key::plain(KeyCode::Enter))),
            vec![Effect::Transaction(TxOp::Commit)]
        );
        type_str(&mut s, ":rollback");
        assert_eq!(
            update(&mut s, Event::Key(Key::plain(KeyCode::Enter))),
            vec![Effect::Transaction(TxOp::Rollback)]
        );
    }

    #[test]
    fn a_transaction_command_is_refused_while_a_query_runs() {
        let mut s = wb();
        submit_query(&mut s, "MATCH (n) RETURN n;"); // now Running
        type_str(&mut s, ":begin");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty(), "no tx op while a query is in flight");
        assert!(s.status.message.contains("busy"), "status: {}", s.status.message);
    }

    // --- :connect (issue 07) ------------------------------------------------

    #[test]
    fn connect_emits_a_connect_effect_and_warns_about_an_open_transaction() {
        let mut s = wb();
        s.tx = mgconsole_core::TransactionState::Open;
        type_str(&mut s, ":connect prod");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(effects, vec![Effect::Connect("prod".to_string())]);
        assert!(s.status.message.contains("aborted by :connect"), "warned: {}", s.status.message);
        assert_eq!(s.editor.buffer(), "");
    }

    #[test]
    fn a_successful_connect_event_updates_the_connection_display() {
        use crate::workbench::event::Connected;
        let mut s = wb();
        s.tx = mgconsole_core::TransactionState::Open;
        update(
            &mut s,
            Event::Connected(Ok(Connected {
                endpoint: "db:7688".to_string(),
                profile: Some("prod".to_string()),
                read_only: true,
                label: "prod (db:7688)".to_string(),
            })),
        );
        assert_eq!(s.endpoint, "db:7688");
        assert_eq!(s.profile.as_deref(), Some("prod"));
        assert!(s.read_only);
        assert_eq!(s.tx, mgconsole_core::TransactionState::Auto);
        assert!(s.status.message.contains("connected to prod"));
    }

    #[test]
    fn a_failed_connect_leaves_the_display_intact() {
        let mut s = wb();
        s.endpoint = "old:7687".to_string();
        update(&mut s, Event::Connected(Err("refused".to_string())));
        assert_eq!(s.endpoint, "old:7687", "prior connection display intact");
        assert!(s.status.message.contains("error"));
    }

    #[test]
    fn source_reads_the_file_then_runs_a_stop_on_error_batch() {
        let mut s = wb();
        type_str(&mut s, ":source setup.cypher");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(effects, vec![Effect::Source(PathBuf::from("setup.cypher"))]);

        // The contents arrive and run as a batch with stop-on-error armed.
        let effects = update(
            &mut s,
            Event::SourceLoaded(Ok("RETURN 1;\nRETURN 2;\n".to_string())),
        );
        match effects.first() {
            Some(Effect::RunQuery { query, .. }) => assert_eq!(query, "RETURN 1"),
            other => panic!("expected RunQuery, got {other:?}"),
        }
        assert_eq!(s.pending.len(), 1);
        assert!(s.source_halt, "stop-on-error armed");

        // A failure halts the batch: the queued statement is dropped.
        let id = match s.run {
            RunState::Running { id } => id,
            RunState::Idle => panic!("should be running"),
        };
        let boom = Error::Query(QueryError {
            code: "X.SyntaxError".to_string(),
            message: "bad".to_string(),
        });
        update(&mut s, Event::QueryFailed { id, error: boom });
        assert!(s.pending.is_empty(), "rest of the batch dropped on error");
        assert!(!s.source_halt, "stop-on-error mode cleared");
        assert!(matches!(s.run, RunState::Idle));
    }

    #[test]
    fn source_of_a_missing_file_reports_an_error() {
        let mut s = wb();
        update(&mut s, Event::SourceLoaded(Err("cannot read x: nope".to_string())));
        assert!(s.status.message.contains("error"), "status: {}", s.status.message);
        assert!(matches!(s.run, RunState::Idle));
    }

    // --- :o redirect (issue 12) ---------------------------------------------

    #[test]
    fn redirect_arms_then_the_next_query_runs_to_file() {
        use crate::OutputFormat;
        let mut s = wb();
        type_str(&mut s, ":o csv /tmp/wb_o.csv");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty(), "arming runs nothing");
        assert!(s.redirect.is_some(), "redirect armed");

        // The next query runs to file (one-shot).
        type_str(&mut s, "RETURN 1;");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        match effects.first() {
            Some(Effect::RunQueryToFile { query, format, path, .. }) => {
                assert_eq!(query, "RETURN 1");
                assert_eq!(*format, OutputFormat::Csv);
                assert_eq!(path, &PathBuf::from("/tmp/wb_o.csv"));
            }
            other => panic!("expected RunQueryToFile, got {other:?}"),
        }
        assert!(s.redirect.is_none(), "one-shot: redirect cleared");

        // The redirected query completes via the Redirected event and the session
        // is idle again.
        let id = match s.run { RunState::Running { id } => id, RunState::Idle => panic!("running") };
        let effects = update(
            &mut s,
            Event::Redirected { id, result: Ok((PathBuf::from("/tmp/wb_o.csv"), 1)) },
        );
        assert!(effects.is_empty());
        assert!(matches!(s.run, RunState::Idle));
        assert!(s.status.message.contains("wrote 1 row"));
    }

    // --- :watch (issue 11) --------------------------------------------------

    #[test]
    fn watch_starts_immediately_and_ticks_re_run_replacing_the_snapshot() {
        let mut s = wb();
        // Establish a last query.
        let id = submit_query(&mut s, "RETURN 1;");
        complete(&mut s, id, 1);
        assert_eq!(s.history.len(), 1);

        // `:watch 1s` starts a fresh run immediately.
        s.editor.clear();
        type_str(&mut s, ":watch 1s");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(matches!(effects.first(), Some(Effect::RunQuery { query, .. }) if query == "RETURN 1"));
        assert!(s.watch.is_some());
        let id = match s.run { RunState::Running { id } => id, RunState::Idle => panic!("running") };
        complete(&mut s, id, 1);
        let after_first = s.history.len();

        // Ticks count down; when the interval elapses a re-run fires, replacing the
        // previous snapshot (history does not grow).
        let period = s.watch.as_ref().unwrap().period_ticks;
        let mut fired = Vec::new();
        for _ in 0..period {
            fired = update(&mut s, Event::Tick);
        }
        assert!(matches!(fired.first(), Some(Effect::RunQuery { .. })), "a re-run fired");
        let id = match s.run { RunState::Running { id } => id, RunState::Idle => panic!("running") };
        complete(&mut s, id, 1);
        assert_eq!(s.history.len(), after_first, "the snapshot was replaced, not appended");
    }

    #[test]
    fn any_key_stops_watching() {
        let mut s = wb();
        let id = submit_query(&mut s, "RETURN 1;");
        complete(&mut s, id, 1);
        s.editor.clear();
        type_str(&mut s, ":watch 1s");
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        let id = match s.run { RunState::Running { id } => id, RunState::Idle => panic!("running") };
        complete(&mut s, id, 1);
        assert!(s.watch.is_some());
        // Any key stops it.
        update(&mut s, Event::Key(Key::char('x')));
        assert!(s.watch.is_none(), "watch stopped");
        assert!(s.status.message.contains("watch stopped"));
    }

    #[test]
    fn watch_is_refused_in_a_transaction() {
        let mut s = wb();
        s.tx = mgconsole_core::TransactionState::Open;
        s.last_query = Some("RETURN 1".to_string());
        type_str(&mut s, ":watch");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty());
        assert!(s.watch.is_none());
        assert!(s.status.message.contains("refused while a transaction is open"));
    }

    #[test]
    fn sysinfo_runs_the_status_queries_as_a_batch() {
        let mut s = wb();
        type_str(&mut s, ":sysinfo");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        // The first status query starts; the rest queue (issue 09).
        match effects.first() {
            Some(Effect::RunQuery { query, .. }) => {
                assert_eq!(query, crate::repl::SYSINFO_QUERIES[0]);
            }
            other => panic!("expected a RunQuery, got {other:?}"),
        }
        assert_eq!(s.pending.len(), crate::repl::SYSINFO_QUERIES.len() - 1);
        assert_eq!(s.editor.buffer(), "");
    }

    #[test]
    fn use_emits_a_use_database_effect() {
        let mut s = wb();
        type_str(&mut s, ":use analytics");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(effects, vec![Effect::UseDatabase("analytics".to_string())]);
        assert_eq!(s.editor.buffer(), "");
    }

    #[test]
    fn the_database_changed_event_updates_the_active_database() {
        let mut s = wb();
        update(&mut s, Event::DatabaseChanged(Ok("analytics".to_string())));
        assert_eq!(s.database.as_deref(), Some("analytics"));
        assert!(s.status.message.contains("using database analytics"));
        // A failed switch keeps the current database.
        update(&mut s, Event::DatabaseChanged(Err("no such db".to_string())));
        assert_eq!(s.database.as_deref(), Some("analytics"), "unchanged on error");
        assert!(s.status.message.contains("error"));
    }

    #[test]
    fn connect_is_refused_while_a_query_runs() {
        let mut s = wb();
        submit_query(&mut s, "MATCH (n) RETURN n;");
        type_str(&mut s, ":connect prod");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty());
        assert!(s.status.message.contains("busy"));
    }

    #[test]
    fn the_transaction_applied_event_updates_the_marker_and_message() {
        use mgconsole_core::TransactionState;
        let mut s = wb();
        update(
            &mut s,
            Event::TransactionApplied {
                state: TransactionState::Open,
                message: Some("transaction open".to_string()),
            },
        );
        assert_eq!(s.tx, TransactionState::Open);
        assert_eq!(s.status.message, "transaction open");
        // A silent marker sync (message None) leaves the status untouched.
        update(
            &mut s,
            Event::TransactionApplied {
                state: TransactionState::Failed,
                message: None,
            },
        );
        assert_eq!(s.tx, TransactionState::Failed);
        assert_eq!(s.status.message, "transaction open", "status untouched by a silent sync");
    }

    // --- persisted history recall (slice 17) --------------------------------

    fn ctrl(code: KeyCode) -> Event {
        Event::Key(Key::ctrl(code))
    }

    #[test]
    fn loaded_history_is_recalled_into_the_editor() {
        let mut s = wb();
        update(
            &mut s,
            Event::HistoryLoaded(vec!["RETURN 1;".to_string(), "MATCH (n) RETURN n;".to_string()]),
        );
        // Ctrl+Up recalls the newest entry, then the older one.
        update(&mut s, ctrl(KeyCode::Up));
        assert_eq!(s.editor.buffer(), "MATCH (n) RETURN n;");
        update(&mut s, ctrl(KeyCode::Up));
        assert_eq!(s.editor.buffer(), "RETURN 1;");
        // Ctrl+Down walks back toward the newest, then restores the (empty) live buffer.
        update(&mut s, ctrl(KeyCode::Down));
        assert_eq!(s.editor.buffer(), "MATCH (n) RETURN n;");
        update(&mut s, ctrl(KeyCode::Down));
        assert_eq!(s.editor.buffer(), "", "stepping past newest restores the live buffer");
    }

    #[test]
    fn a_submitted_query_is_appended_to_history_and_persisted() {
        let mut s = wb();
        type_str(&mut s, "RETURN 7;");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(
            effects.iter().any(|e| matches!(e, Effect::AppendHistory(line) if line == "RETURN 7;")),
            "submission persisted: {effects:?}"
        );
        assert_eq!(s.history_entries, vec!["RETURN 7;".to_string()], "appended in-memory");
    }

    #[test]
    fn the_live_buffer_is_preserved_across_recall() {
        let mut s = wb();
        update(&mut s, Event::HistoryLoaded(vec!["OLD;".to_string()]));
        type_str(&mut s, "typing");
        update(&mut s, ctrl(KeyCode::Up)); // recall OLD, saving "typing"
        assert_eq!(s.editor.buffer(), "OLD;");
        update(&mut s, ctrl(KeyCode::Down)); // past newest → restore
        assert_eq!(s.editor.buffer(), "typing");
    }

    // --- notifications / stats / summary (slice 18) -------------------------

    #[test]
    fn a_completed_result_keeps_its_summary_and_notes_notifications_in_the_status() {
        use mgconsole_core::Notification;
        let mut s = wb();
        let id = submit_query(&mut s, "MATCH (n) RETURN n;");
        update(&mut s, Event::QueryStarted { id, header: vec!["n".to_string()] });
        let summary = Summary {
            notifications: vec![Notification {
                code: "Hint".into(),
                title: "Add an index".into(),
                description: "label scan".into(),
                severity: "INFORMATION".into(),
            }],
            ..Summary::default()
        };
        update(
            &mut s,
            Event::QueryCompleted { id, summary, elapsed: Duration::from_millis(1) },
        );
        assert!(s.status.message.contains("1 notification"), "status: {}", s.status.message);
        assert!(s.shown().unwrap().summary.is_some(), "summary kept for the drawer");
    }

    #[test]
    fn ctrl_y_toggles_the_summary_drawer() {
        let mut s = wb();
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('y'))));
        assert_eq!(s.drawer, Some(DrawerKind::Summary));
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('y'))));
        assert_eq!(s.drawer, None);
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

    // --- Named queries (issue 13): the same verbs as the REPL, reducer seam ----

    /// Submit a `:`-meta command by typing it and pressing Enter, returning the
    /// effects. Clears the editor first (a prior submitted query keeps its text).
    fn submit_meta(state: &mut WorkbenchState, command: &str) -> Vec<Effect> {
        state.editor.clear();
        type_str(state, command);
        update(state, Event::Key(Key::plain(KeyCode::Enter)))
    }

    #[test]
    fn save_with_a_query_stores_a_template_and_requests_a_persist() {
        let mut s = wb();
        let effects = submit_meta(&mut s, ":save recent MATCH (n) RETURN $limit");
        // The $param placeholder is kept verbatim — a template, not a frozen value.
        assert_eq!(s.queries.get("recent"), Some("MATCH (n) RETURN $limit"));
        assert_eq!(effects, vec![Effect::PersistQueries]);
        assert!(s.status.message.contains("saved 'recent'"));
        assert_eq!(s.editor.buffer(), "", "a command is consumed");
    }

    #[test]
    fn save_with_no_query_saves_the_last_query() {
        let mut s = wb();
        submit_query(&mut s, "RETURN 1;");
        let effects = submit_meta(&mut s, ":save one");
        assert_eq!(s.queries.get("one"), Some("RETURN 1"));
        assert_eq!(effects, vec![Effect::PersistQueries]);
    }

    #[test]
    fn save_with_no_query_and_no_last_query_is_reported() {
        let mut s = wb();
        let effects = submit_meta(&mut s, ":save one");
        assert!(effects.is_empty(), "nothing persisted");
        assert!(s.status.message.contains("no query to save"));
        assert!(s.queries.is_empty());
    }

    #[test]
    fn saved_summarises_the_store_into_the_status_bar() {
        let mut s = wb();
        s.queries.set("a".to_string(), "RETURN 1".to_string());
        s.queries.set("b".to_string(), "RETURN 2".to_string());
        submit_meta(&mut s, ":saved");
        assert_eq!(s.status.message, "saved: a, b");
    }

    #[test]
    fn load_recalls_the_template_into_the_editor_and_does_not_run_it() {
        let mut s = wb();
        s.queries
            .set("recent".to_string(), "MATCH (n) RETURN $limit".to_string());
        let effects = submit_meta(&mut s, ":load recent");
        // Recall to input: the editor now holds the template, ready to edit/submit.
        assert_eq!(s.editor.buffer(), "MATCH (n) RETURN $limit");
        assert!(effects.is_empty(), "load never runs the query");
        assert!(matches!(s.run, RunState::Idle));
    }

    #[test]
    fn load_of_an_unknown_name_is_reported() {
        let mut s = wb();
        let effects = submit_meta(&mut s, ":load nope");
        assert!(effects.is_empty());
        assert!(s.status.message.contains("no saved query named 'nope'"));
    }

    #[test]
    fn forget_removes_a_saved_query_and_an_unknown_name_is_reported() {
        let mut s = wb();
        s.queries.set("a".to_string(), "RETURN 1".to_string());
        let effects = submit_meta(&mut s, ":forget a");
        assert!(s.queries.get("a").is_none());
        assert_eq!(effects, vec![Effect::PersistQueries]);
        assert!(s.status.message.contains("forgot 'a'"));

        let effects = submit_meta(&mut s, ":forget gone");
        assert!(effects.is_empty(), "nothing to persist for an absent name");
        assert!(s.status.message.contains("no saved query named 'gone'"));
    }

    // --- Theme + keybindings (issue 14) ---------------------------------------

    #[test]
    fn set_theme_repaints_the_palette_at_runtime() {
        use crate::syntax::HighlightCategory;
        use crate::theme::ThemeColor;
        let mut s = wb();
        // Default theme: keyword is yellow.
        assert_eq!(s.palette.color(HighlightCategory::Keyword), ThemeColor::Yellow);
        submit_meta(&mut s, ":set theme mono");
        // mono: every category the terminal default.
        assert_eq!(s.palette.color(HighlightCategory::Keyword), ThemeColor::Default);
        assert_eq!(s.status.message, "theme = mono");
    }

    #[test]
    fn set_theme_keeps_config_overrides_over_the_new_base() {
        use crate::syntax::HighlightCategory;
        use crate::theme::ThemeColor;
        let mut config = WorkbenchConfig::default();
        // A config [theme] override sits on top of whichever base is active.
        config
            .theme_overrides
            .insert("keyword".to_string(), "red".to_string());
        let (palette, _) = crate::theme::resolve_palette("default", &config.theme_overrides);
        config.palette = palette;
        let mut s = WorkbenchState::new(config, true);
        assert_eq!(s.palette.color(HighlightCategory::Keyword), ThemeColor::Red);
        // Switching the base to mono keeps the explicit keyword override.
        submit_meta(&mut s, ":set theme mono");
        assert_eq!(s.palette.color(HighlightCategory::Keyword), ThemeColor::Red);
        assert_eq!(s.palette.color(HighlightCategory::Function), ThemeColor::Default);
    }

    #[test]
    fn set_theme_lists_alongside_the_other_settings() {
        let mut s = wb();
        submit_meta(&mut s, ":set");
        assert!(s.status.message.contains("theme = default"), "{}", s.status.message);
    }

    #[test]
    fn a_default_chord_drives_its_gesture() {
        // Ctrl-P toggles the params drawer by default (no rebinding).
        let mut s = wb();
        assert_eq!(s.drawer, None);
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('p'))));
        assert_eq!(s.drawer, Some(DrawerKind::Params));
    }

    #[test]
    fn a_rebound_chord_drives_the_gesture_and_the_old_chord_does_not() {
        // Rebind toggle-params to Ctrl-G; the default Ctrl-P no longer toggles it.
        let mut config = WorkbenchConfig::default();
        let (keys, warnings) = crate::theme::resolve_keys(&BTreeMap::from([(
            "toggle-params".to_string(),
            "ctrl+g".to_string(),
        )]));
        assert!(warnings.is_empty());
        config.keys = keys;
        let mut s = WorkbenchState::new(config, true);

        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('g'))));
        assert_eq!(s.drawer, Some(DrawerKind::Params), "the rebound chord works");

        // The old default chord is now unbound: Ctrl-P is ordinary input, not a toggle.
        s.drawer = None;
        s.focus = Focus::Editor;
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('p'))));
        assert_eq!(s.drawer, None, "the freed default chord no longer toggles");
    }
}
