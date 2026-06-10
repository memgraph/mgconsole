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

use mgconsole_core::{Error, QueryAssembler, Record, Summary, Value};

use crate::repl::{format_summary, meta_command, MetaCommand};
use crate::syntax::Completer;
use crate::theme::{Chord, ChordKey, Gesture, KeyBindings};

use super::effect::{Effect, TxOp};
use super::event::{Event, Key, KeyCode, MouseEvent, MouseKind};
use super::plan::{is_plan_query, Plan};
use super::schema::{Schema, SchemaSource};
use super::state::{
    Completion, CurrentResult, DrawerKind, ExportPrompt, Focus, RunState, SearchState, WatchState,
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
        Event::Mouse(mouse) => update_mouse(state, mouse),
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

/// A query began: push a fresh result onto its *origin* Buffer's history stack
/// (issue 03) for its rows to stream into, and show it there (auto-follow). The
/// origin Buffer may not be the active one — the user may have switched away —
/// so the result lands in the Buffer the query was submitted from, never the
/// active Buffer's history.
fn on_started(state: &mut WorkbenchState, id: u64, header: Vec<String>) {
    if !is_current(state, id) {
        return;
    }
    state.running_started = true;
    let statement = state.running_statement.clone().unwrap_or_default();
    let cap = state.config.history_cap;
    if let Some((history, view)) = state.running_target() {
        history.push(CurrentResult::new(statement, header));
        *view = history.len() - 1;
        trim_history(history, cap);
    }
}

/// Bound a Buffer's Result history to `cap` full entries (issue 06): every entry
/// older than the most recent `cap` keeps its query / summary / error but drops
/// its Records (the memory cost), marked `trimmed` so the draw shows a "rows no
/// longer held" note. The newest `cap` entries always keep their Records.
fn trim_history(history: &mut [CurrentResult], cap: usize) {
    if history.len() <= cap {
        return;
    }
    let keep_from = history.len() - cap;
    for entry in &mut history[..keep_from] {
        entry.rows = Vec::new();
        entry.trimmed = true;
    }
}

/// A record streamed in: append it to the origin Buffer's live (last) result, up
/// to the row-cap backstop (beyond which rows are dropped and the result marked
/// truncated — a memory guard, not a usability limit).
fn on_record(state: &mut WorkbenchState, id: u64, record: Record) {
    if !is_current(state, id) {
        return;
    }
    let cap = state.config.row_cap;
    if let Some((history, _)) = state.running_target() {
        if let Some(result) = history.last_mut() {
            if result.rows.len() < cap {
                result.rows.push(record);
            } else {
                result.truncated = true;
            }
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
    // Surface a notification count in the status line (full detail in the
    // summary drawer, slice 18); the drawer reads the stored Summary. Computed
    // before the summary is moved into the origin Buffer's result.
    let note = match summary.notifications.len() {
        0 => String::new(),
        1 => " · 1 notification".to_string(),
        n => format!(" · {n} notifications"),
    };
    let mut rows = 0;
    if let Some((history, _)) = state.running_target() {
        if let Some(result) = history.last_mut() {
            rows = result.rows.len();
            result.summary = Some(summary);
            // An EXPLAIN/PROFILE result renders as an operator tree, not a table.
            if is_plan_query(&result.statement) {
                result.plan = Some(Plan::parse(&result.rows));
            }
        }
    }
    state.status.message = format!("{}{note}", format_summary(rows, elapsed));
    advance(state)
}

/// The query failed: surface the error without losing the Session (ADR 0005),
/// record it as a navigable Result-history entry (issue 05), then continue with
/// the next pending statement (as the REPL does mid-batch).
///
/// A failure makes its submission exactly one entry pairing the query with the
/// error: if the query had already started (so an entry exists, possibly with
/// streamed rows), the error lands on that entry; if it failed before any Record
/// — the common case, e.g. a syntax error — a fresh failure entry is pushed
/// carrying the originating query and the error, so it does not vanish as a
/// transient status.
fn on_failed(state: &mut WorkbenchState, id: u64, error: &Error) -> Vec<Effect> {
    if !is_current(state, id) {
        return Vec::new();
    }
    let error_text = error.to_string();
    let statement = state.running_statement.clone().unwrap_or_default();
    let started = state.running_started;
    let cap = state.config.history_cap;
    if let Some((history, view)) = state.running_target() {
        if started {
            if let Some(result) = history.last_mut() {
                result.error = Some(error_text.clone());
            }
        } else {
            let mut result = CurrentResult::new(statement, Vec::new());
            result.error = Some(error_text.clone());
            history.push(result);
            *view = history.len() - 1;
            trim_history(history, cap);
        }
    }
    state.status.message = format!("error: {error_text}");
    // A `:source` batch stops on the first error (issue 10): drop the rest.
    if state.source_halt {
        state.source_halt = false;
        state.pending.clear();
        state.run = RunState::Idle;
        state.running_buffer = None;
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
        // The whole submission's batch is done: release its origin Buffer (issue 03).
        state.running_buffer = None;
        // The `:source` batch (issue 10) finished cleanly; leave stop-on-error mode.
        state.source_halt = false;
        Vec::new()
    }
}

/// Begin running `query`: stamp it with a fresh id, mark the Session busy, and
/// emit the run effect with the bound parameters. The first statement of a
/// submission claims its origin Buffer (issue 03); the rest of the batch keeps it.
fn start_query(state: &mut WorkbenchState, query: String) -> Vec<Effect> {
    let id = state.next_id;
    state.next_id += 1;
    state.run = RunState::Running { id };
    state.spinner = 0;
    state.running_started = false;
    if state.running_buffer.is_none() {
        state.running_buffer = Some(state.active);
    }
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
        // Mark the origin Buffer's partial result, even if another Buffer is shown.
        let mut rows = 0;
        if let Some((history, _)) = state.running_target() {
            if let Some(result) = history.last_mut() {
                rows = result.rows.len();
                result.partial = true;
            }
        }
        state.pending.clear();
        state.run = RunState::Idle;
        state.running_buffer = None;
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
    if state.help {
        return help_key(state, key);
    }
    if state.detail.is_some() {
        return detail_key(state, key);
    }
    if state.export.is_some() {
        return export_key(state, key);
    }
    // In-result search captures keys while open (issue 16): typing edits the query,
    // arrows step matches, Tab toggles the filter, Esc clears and closes.
    if state.search.is_some() {
        return search_key(state, key);
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

/// Handle a mouse event (issue 17) by translating it into the existing reducer
/// paths — focus, selection, cell-expand, and scroll — so there is no parallel
/// mouse state. Hit-testing uses the pane rectangles the draw cached. While a
/// modal overlay (cell-detail, export, completion) is open, the mouse is ignored
/// so it never fights the keyboard-driven overlay.
fn update_mouse(state: &mut WorkbenchState, mouse: MouseEvent) -> Vec<Effect> {
    if state.help
        || state.detail.is_some()
        || state.export.is_some()
        || state.completion.is_some()
    {
        return Vec::new();
    }
    let (col, row) = (mouse.column, mouse.row);
    match mouse.kind {
        // Scroll over a pane drives that pane: result rows (the existing row-nav
        // path) or the editor cursor (which scrolls its viewport).
        MouseKind::ScrollUp | MouseKind::ScrollDown => {
            let down = matches!(mouse.kind, MouseKind::ScrollDown);
            if in_rect(state.results_area, col, row) {
                return results_key(state, Key::plain(if down { KeyCode::Down } else { KeyCode::Up }));
            }
            if in_rect(state.editor_area, col, row) {
                state.editor.edit(Key::plain(if down { KeyCode::Down } else { KeyCode::Up }));
            }
            Vec::new()
        }
        // A click on the tab bar switches Buffers (issue 18); on a pane it focuses
        // it, and over a result cell selects that cell and opens the cell-detail
        // overlay (the existing cell-expand path).
        MouseKind::Down => {
            if in_rect(state.tabbar_area, col, row) {
                click_tab(state, col);
            } else if in_rect(state.results_area, col, row) {
                state.focus = Focus::Results;
                click_result_cell(state, col, row);
            } else if in_rect(state.editor_area, col, row) {
                state.focus = Focus::Editor;
            }
            Vec::new()
        }
    }
}

/// Switch to the Buffer whose tab a click landed on (issue 18). Tabs are
/// fixed-width, so the index is `(click - bar.x) / TAB_WIDTH`; a click past the
/// last tab is ignored. Allowed while a query is live (issue 03): the live query
/// streams into its origin Buffer regardless of which is shown.
fn click_tab(state: &mut WorkbenchState, col: u16) {
    let offset = col.saturating_sub(state.tabbar_area.x);
    let index = (offset / super::draw::TAB_WIDTH) as usize;
    if index < state.buffer_count() && index != state.active {
        state.switch_to(index);
        state.status.message = format!("buffer {}/{}", state.active + 1, state.buffer_count());
    }
}

/// Whether `(col, row)` falls inside `rect`.
fn in_rect(rect: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

/// Select the result cell a click landed on and open its cell-detail overlay
/// (issue 17 → the slice-08 cell-expand path). The header occupies the first row
/// of the results rect; columns share the width equally. A click on the header row
/// or below the loaded rows only focuses the pane (handled by the caller). Skipped
/// while the filtered search view is active, where the visible rows are a subset.
fn click_result_cell(state: &mut WorkbenchState, col: u16, row: u16) {
    if state.search.as_ref().is_some_and(|s| s.filter_only) {
        return;
    }
    let area = state.results_area;
    // The first row is the pinned header; data rows start one below it.
    if row <= area.y {
        return;
    }
    let row_in_view = (row - area.y - 1) as usize;
    let Some(result) = state.shown() else {
        return;
    };
    if result.plan.is_some() || result.header.is_empty() {
        return;
    }
    let ncols = result.header.len();
    let absolute = result.scroll + row_in_view;
    if absolute >= result.rows.len() {
        return; // a click below the last loaded row only focuses the pane
    }
    let rel = (col - area.x) as usize;
    let width = area.width.max(1) as usize;
    let column = (rel * ncols / width).min(ncols - 1);
    if let Some(result) = state.shown_mut() {
        result.selected_row = absolute;
        result.selected_col = column;
    }
    open_detail(state);
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
        // Auto-format the editor's Cypher (issue 15): a pure text->text reformat
        // over the Core lexer's tokens. Partial/unsafe input is left untouched with
        // a clear status (the formatter declines rather than mangling).
        Gesture::FormatBuffer => {
            match crate::cypher_format::format(&state.editor.buffer()) {
                Ok(formatted) => {
                    state.editor.set_text(&formatted);
                    state.status.message = "formatted".to_string();
                }
                Err(reason) => state.status.message = reason,
            }
            Vec::new()
        }
        Gesture::Search => {
            open_search(state);
            Vec::new()
        }
        // Buffer (tab) navigation gestures (issue 18). Allowed while a query is
        // live (issue 03): a live query belongs to its origin Buffer and streams
        // there regardless of which Buffer is shown, so switching to read another
        // line of inquiry never misroutes — only a second submit is refused.
        // Closing a Buffer is the `:close` command, not a gesture (CONTEXT.md).
        Gesture::NewBuffer | Gesture::NextBuffer | Gesture::PrevBuffer => {
            match gesture {
                Gesture::NewBuffer => {
                    state.new_buffer();
                    state.status.message =
                        format!("new buffer {}/{}", state.active + 1, state.buffer_count());
                }
                Gesture::NextBuffer => {
                    state.cycle_buffer(true);
                    state.status.message =
                        format!("buffer {}/{}", state.active + 1, state.buffer_count());
                }
                _ => {
                    state.cycle_buffer(false);
                    state.status.message =
                        format!("buffer {}/{}", state.active + 1, state.buffer_count());
                }
            }
            Vec::new()
        }
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

/// Open in-result search over the shown result (issue 16). Refused with a status
/// when there is no result with rows to search; when the result is partial or
/// truncated, the status states that search covers only the loaded rows.
fn open_search(state: &mut WorkbenchState) {
    let Some(result) = state.shown() else {
        state.status.message = "no result to search".to_string();
        return;
    };
    if result.rows.is_empty() {
        state.status.message = "no rows to search".to_string();
        return;
    }
    let partial = result.partial || result.truncated;
    state.search = Some(SearchState::default());
    state.status.message = if partial {
        "search (loaded rows only): type to match, ↑/↓ next/prev, Tab filter, Esc clear"
            .to_string()
    } else {
        "search: type to match, ↑/↓ next/prev, Tab filter, Esc clear".to_string()
    };
}

/// Keys while in-result search is open (issue 16): edit the query (live matches),
/// step between matches, toggle the filtered view, or clear and close.
fn search_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    match key {
        Key { code: KeyCode::Esc, .. } => {
            // Clearing search restores the full view (the filter lives in the
            // search state, so dropping it un-filters).
            state.search = None;
            state.status.message = "search cleared".to_string();
        }
        Key { code: KeyCode::Backspace, .. } => {
            if let Some(search) = state.search.as_mut() {
                search.query.pop();
            }
            recompute_search(state);
        }
        // Tab toggles the filtered view (only matching rows) on/off.
        Key { code: KeyCode::Tab, .. } => {
            if let Some(search) = state.search.as_mut() {
                search.filter_only = !search.filter_only;
            }
            update_search_status(state);
        }
        // Enter / Down step to the next match; Up to the previous (wrapping).
        Key { code: KeyCode::Enter | KeyCode::Down, .. } => search_step(state, 1),
        Key { code: KeyCode::Up, .. } => search_step(state, -1),
        // Ordinary printable input extends the query and re-matches live.
        Key { code: KeyCode::Char(c), ctrl: false, alt: false, .. } => {
            if let Some(search) = state.search.as_mut() {
                search.query.push(c);
            }
            recompute_search(state);
        }
        _ => {}
    }
    Vec::new()
}

/// Whether any cell of `record` contains `needle` (a lowercased substring),
/// rendered exactly as the table shows it so the highlight matches the test.
fn row_matches(record: &Record, needle: &str) -> bool {
    record
        .fields()
        .iter()
        .any(|value| mgconsole_core::render::tabular(value).to_lowercase().contains(needle))
}

/// Recompute the match set after the query changed, reset the active match to the
/// first, and move the selection onto it.
fn recompute_search(state: &mut WorkbenchState) {
    let needle = state
        .search
        .as_ref()
        .map(|s| s.query.to_lowercase())
        .unwrap_or_default();
    let matches: Vec<usize> = if needle.is_empty() {
        Vec::new()
    } else if let Some(result) = state.shown() {
        result
            .rows
            .iter()
            .enumerate()
            .filter(|(_, record)| row_matches(record, &needle))
            .map(|(index, _)| index)
            .collect()
    } else {
        Vec::new()
    };
    if let Some(search) = state.search.as_mut() {
        search.current = (!matches.is_empty()).then_some(0);
        search.matches = matches;
    }
    move_selection_to_match(state);
    update_search_status(state);
}

/// Step the active match by `delta` (wrapping) and move the selection onto it.
fn search_step(state: &mut WorkbenchState, delta: isize) {
    if let Some(search) = state.search.as_mut() {
        if search.matches.is_empty() {
            return;
        }
        let len = search.matches.len() as isize;
        let current = search.current.unwrap_or(0) as isize;
        search.current = Some((current + delta).rem_euclid(len) as usize);
    }
    move_selection_to_match(state);
    update_search_status(state);
}

/// Move the result's selection onto the active match and keep it in the viewport.
fn move_selection_to_match(state: &mut WorkbenchState) {
    let target = state
        .search
        .as_ref()
        .and_then(|s| s.current.map(|i| s.matches[i]));
    let Some(row) = target else {
        return;
    };
    let page = state.viewport_rows.max(1);
    if let Some(result) = state.shown_mut() {
        result.selected_row = row;
        if result.selected_row < result.scroll {
            result.scroll = result.selected_row;
        } else if result.selected_row >= result.scroll + page {
            result.scroll = result.selected_row + 1 - page;
        }
    }
}

/// Refresh the status line with the current match count and filter state.
fn update_search_status(state: &mut WorkbenchState) {
    let Some(search) = state.search.as_ref() else {
        return;
    };
    let filter = if search.filter_only { " · filtered" } else { "" };
    state.status.message = if search.query.is_empty() {
        format!("search:{filter}")
    } else if search.matches.is_empty() {
        format!("search '{}' — no matches{filter}", search.query)
    } else {
        let position = search.current.map_or(0, |i| i + 1);
        format!(
            "search '{}' — {}/{}{filter}",
            search.query,
            position,
            search.matches.len()
        )
    };
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
        // Vim-style yank (issue 04 / ADR 0015): 'y' copies the selected cell's
        // rendered Value, 'Y' the whole selected row, each via an OSC 52 effect.
        // (`Ctrl+C` is cancel, hence the lowercase/uppercase letters.)
        KeyCode::Char('y') => return yank_cell(state),
        KeyCode::Char('Y') => return yank_row(state),
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
    // On a failed entry there are no cells; cell-expand shows the originating
    // query in full instead (issue 05), so a truncated header snippet is never
    // the only view of what ran.
    if let Some(result) = state.shown() {
        if result.error.is_some() {
            let statement = result.statement.clone();
            state.detail = Some(Value::String(statement));
            state.detail_scroll = 0;
            return;
        }
    }
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

/// Yank the selected cell's rendered Value to the clipboard (issue 04 / ADR
/// 0015): the exact text shown, copied via OSC 52. A plan result or an empty
/// table has no cell to copy, so the yank is a no-op.
fn yank_cell(state: &mut WorkbenchState) -> Vec<Effect> {
    let text = state.shown().and_then(|result| {
        if result.plan.is_some() {
            return None;
        }
        result
            .rows
            .get(result.selected_row)
            .and_then(|row| row.fields().get(result.selected_col))
            .map(mgconsole_core::render::tabular)
    });
    match text {
        Some(text) => {
            state.status.message = "copied cell".to_string();
            vec![Effect::CopyToClipboard(text)]
        }
        None => Vec::new(),
    }
}

/// Yank the whole selected row to the clipboard (issue 04 / ADR 0015): its cells
/// rendered as shown and joined by tabs, so it pastes as one row. A no-op for a
/// plan result or an empty table.
fn yank_row(state: &mut WorkbenchState) -> Vec<Effect> {
    let text = state.shown().and_then(|result| {
        if result.plan.is_some() {
            return None;
        }
        result.rows.get(result.selected_row).map(|row| {
            row.fields()
                .iter()
                .map(mgconsole_core::render::tabular)
                .collect::<Vec<_>>()
                .join("\t")
        })
    });
    match text {
        Some(text) => {
            state.status.message = "copied row".to_string();
            vec![Effect::CopyToClipboard(text)]
        }
        None => Vec::new(),
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

/// Keys while the `:help` overlay is open: scroll it, or dismiss it.
fn help_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    match key.code {
        KeyCode::Up | KeyCode::PageUp => {
            state.help_scroll = state.help_scroll.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::PageDown => {
            state.help_scroll = state.help_scroll.saturating_add(1);
        }
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => state.help = false,
        _ => {}
    }
    Vec::new()
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
        // The cell-detail overlay yanks the same way as the results pane (issue
        // 04 / ADR 0015): 'y' copies the full Value shown via OSC 52.
        KeyCode::Char('y') => {
            if let Some(value) = state.detail.as_ref() {
                let text = mgconsole_core::render::tabular(value);
                state.status.message = "copied value".to_string();
                return vec![Effect::CopyToClipboard(text)];
            }
        }
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
    state.running_started = false;
    if state.running_buffer.is_none() {
        state.running_buffer = Some(state.active);
    }
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
                "{} · readonly = {} · mouse = {}",
                state.settings.list().replace('\n', " · "),
                crate::repl::on_off(state.read_only),
                crate::repl::on_off(state.mouse),
            );
            state.editor.clear();
            Vec::new()
        }
        // `mouse` (issue 04) is a Workbench-only session toggle, not a precedence-
        // resolved Setting: `:set mouse off` releases mouse capture so native
        // selection works; `:set mouse on` restores the Workbench gestures.
        MetaCommand::SetSetting { name, value } if name == "mouse" => {
            state.editor.clear();
            match crate::repl::parse_on_off(&value) {
                Ok(on) => {
                    state.mouse = on;
                    state.status.message = format!("mouse = {}", crate::repl::on_off(on));
                    return vec![Effect::SetMouseCapture(on)];
                }
                Err(message) => state.status.message = format!("error: {message}"),
            }
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
        // `:close` is the deliberate, typed way to close the active Buffer — the
        // destructive counterpart to the navigation gestures (CONTEXT.md). The
        // last Buffer always stays open; refused while a query is live.
        MetaCommand::Close => {
            state.editor.clear();
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
            } else if state.buffer_count() == 1 {
                state.status.message = "the last buffer stays open".to_string();
            } else {
                state.close_buffer();
                state.status.message =
                    format!("buffer {}/{}", state.active + 1, state.buffer_count());
            }
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
        // `:help`/`:docs` open the keybinding + command overlay (its chords reflect
        // the live `[keys]` bindings). Dismissed by Esc/Enter/q.
        MetaCommand::Help | MetaCommand::Docs => {
            state.help = true;
            state.help_scroll = 0;
            state.editor.clear();
            Vec::new()
        }
    }
}

/// The `:help` overlay text (issue 14/18 follow-up): every gesture with its
/// *current* chord, so a `[keys]` rebinding shows the real binding rather than the
/// default. Fixed (non-rebindable) gestures are listed verbatim, then the
/// `:`-command reference. Pure over the [`KeyBindings`], so it is unit-tested.
pub fn keybindings_help(keys: &KeyBindings) -> String {
    use std::fmt::Write as _;
    // A rebindable gesture's live chord, formatted as it appears in `[keys]`.
    let chord = |g: Gesture| keys.chord(g).to_string();
    let mut out = String::new();
    let section = |out: &mut String, title: &str, rows: &[(String, &str)]| {
        out.push_str(title);
        out.push('\n');
        for (key, desc) in rows {
            // Left-pad the key column to a fixed width for an aligned table.
            let _ = writeln!(out, "  {key:<18}{desc}");
        }
        out.push('\n');
    };

    section(
        &mut out,
        "Editing & running",
        &[
            ("Enter".to_string(), "Run the query in the editor"),
            ("Alt+Enter / Ctrl+J".to_string(), "Insert a newline (also Shift/Ctrl+Enter)"),
            ("Tab".to_string(), "Complete the word, or switch pane focus"),
            ("Ctrl+Up / Ctrl+Down".to_string(), "Recall older / newer query (command history)"),
            ("Ctrl+C".to_string(), "Cancel a running query, or clear the editor"),
            ("Esc / Ctrl+D".to_string(), "Quit the workbench"),
        ],
    );
    section(
        &mut out,
        "Results",
        &[
            ("[ / ]".to_string(), "Previous / next result (result history)"),
            ("Up/Down/PgUp/PgDn".to_string(), "Move the row selection"),
            ("Left / Right".to_string(), "Move the column selection"),
            ("Enter".to_string(), "Expand the selected cell"),
            ("y / Y".to_string(), "Yank the selected cell / row to the clipboard"),
            ("e".to_string(), "Export the on-screen result"),
            (chord(Gesture::Search), "Search / filter rows in the result"),
        ],
    );
    section(
        &mut out,
        "Buffers (tabs)",
        &[
            (chord(Gesture::NewBuffer), "New tab"),
            (chord(Gesture::NextBuffer), "Next tab"),
            (chord(Gesture::PrevBuffer), "Previous tab"),
            (":close".to_string(), "Close the active tab (a typed command)"),
        ],
    );
    section(
        &mut out,
        "Drawers & tools",
        &[
            (chord(Gesture::ToggleSchema), "Toggle the schema sidebar"),
            (chord(Gesture::ToggleParams), "Toggle the parameters drawer"),
            (chord(Gesture::ToggleSummary), "Toggle the notifications/stats drawer"),
            (chord(Gesture::RefreshSchema), "Refresh the schema"),
            (chord(Gesture::FormatBuffer), "Auto-format the Cypher in the editor"),
        ],
    );
    out.push_str(
        "Commands (type at the editor)\n  \
         :param :params · :set · :begin :commit :rollback · :connect :use · :sysinfo\n  \
         :source :watch :o · :save :saved :load :forget · :close · :help :docs :quit\n\n\
         Tab chords and tool chords are rebindable in ~/.mgconsole/config.toml under [keys].",
    );
    out
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

    // --- Correlated Result history (issue 05) ---------------------------------

    fn boom() -> Error {
        Error::Query(QueryError {
            code: "Memgraph.ClientError.MemgraphError.SyntaxError".to_string(),
            message: "bad cypher".to_string(),
        })
    }

    #[test]
    fn a_failure_before_any_record_becomes_one_navigable_entry() {
        let mut s = wb();
        // A syntax error fails before QueryStarted: only QueryFailed arrives.
        let id = submit_query(&mut s, "BAD;");
        update(&mut s, Event::QueryFailed { id, error: boom() });
        assert_eq!(s.history.len(), 1, "exactly one history entry for the submission");
        let entry = &s.history[0];
        // The assembler strips the trailing `;`, so the stored statement is "BAD".
        assert_eq!(entry.statement, "BAD", "carries its originating query");
        assert!(entry.error.as_deref().unwrap_or("").contains("bad cypher"), "carries the error");
        assert!(entry.rows.is_empty(), "no records — the error is in their place");
        assert_eq!(s.view, 0, "the failure entry is shown");
    }

    #[test]
    fn a_failure_after_streaming_sets_the_error_on_the_started_entry() {
        let mut s = wb();
        let id = submit_query(&mut s, "MATCH (n) RETURN n;");
        // The query starts and streams a row, then fails mid-stream.
        update(&mut s, Event::QueryStarted { id, header: vec!["n".to_string()] });
        update(&mut s, Event::RecordArrived { id, record: one_row() });
        update(&mut s, Event::QueryFailed { id, error: boom() });
        assert_eq!(s.history.len(), 1, "still exactly one entry (no duplicate)");
        let entry = &s.history[0];
        assert_eq!(entry.rows.len(), 1, "the streamed row is kept");
        assert!(entry.error.is_some(), "the error is recorded on that entry");
    }

    #[test]
    fn each_submission_correlates_its_own_query_across_a_failing_batch() {
        let mut s = wb();
        // Two statements: the first succeeds, the second fails before starting.
        let id0 = submit_query(&mut s, "RETURN 1;\nBAD;");
        update(&mut s, Event::QueryStarted { id: id0, header: vec!["n".to_string()] });
        let effects = update(
            &mut s,
            Event::QueryCompleted { id: id0, summary: Summary::default(), elapsed: Duration::from_millis(1) },
        );
        // Completing the first advances to the second.
        let id1 = match effects.first() {
            Some(Effect::RunQuery { id, .. }) => *id,
            other => panic!("expected the next statement to run, got {other:?}"),
        };
        update(&mut s, Event::QueryFailed { id: id1, error: boom() });
        assert_eq!(s.history.len(), 2, "one entry per statement");
        assert_eq!(s.history[0].statement, "RETURN 1");
        assert!(s.history[0].error.is_none(), "the first succeeded");
        assert_eq!(s.history[1].statement, "BAD");
        assert!(s.history[1].error.is_some(), "the second failed");
    }

    #[test]
    fn cell_expand_on_a_failed_entry_shows_the_full_query() {
        let mut s = wb();
        let id = submit_query(&mut s, "MATCH (n) WHERE n.x = 1 RETURN n;");
        update(&mut s, Event::QueryFailed { id, error: boom() });
        // Enter on the failed entry (results pane focused) expands the query in full.
        s.focus = Focus::Results;
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        match &s.detail {
            Some(Value::String(q)) => assert!(q.contains("MATCH (n) WHERE n.x = 1"), "full query: {q}"),
            other => panic!("expected the full query in the detail overlay, got {other:?}"),
        }
    }

    // --- Bounded Result history (issue 06) ------------------------------------

    /// Run `query` to completion (start → one row → complete) on an idle session.
    fn run_to_completion(state: &mut WorkbenchState, query: &str) {
        state.editor.clear(); // submit keeps the editor text; clear leftover first
        let id = submit_query(state, query);
        update(state, Event::QueryStarted { id, header: vec!["n".to_string()] });
        update(state, Event::RecordArrived { id, record: one_row() });
        update(
            state,
            Event::QueryCompleted { id, summary: Summary::default(), elapsed: Duration::from_millis(1) },
        );
    }

    #[test]
    fn the_history_is_bounded_dropping_records_of_older_entries() {
        let mut s = wb();
        s.config.history_cap = 2; // keep only the newest two full entries
        run_to_completion(&mut s, "RETURN 1;");
        run_to_completion(&mut s, "RETURN 2;");
        run_to_completion(&mut s, "RETURN 3;");
        assert_eq!(s.history.len(), 3, "all entries stay navigable");
        // The oldest entry kept its correlation but dropped its Records.
        assert_eq!(s.history[0].statement, "RETURN 1");
        assert!(s.history[0].trimmed, "the oldest entry is trimmed");
        assert!(s.history[0].rows.is_empty(), "its Records were dropped");
        // The newest two keep their Records.
        assert!(!s.history[1].trimmed && s.history[1].rows.len() == 1, "newest-but-one kept");
        assert!(!s.history[2].trimmed && s.history[2].rows.len() == 1, "newest kept");
    }

    #[test]
    fn a_trimmed_entry_keeps_its_query_and_error_correlation() {
        let mut s = wb();
        s.config.history_cap = 1;
        // A failed submission, then a successful one pushes it past the cap.
        let id = submit_query(&mut s, "BAD;");
        update(&mut s, Event::QueryFailed { id, error: boom() });
        run_to_completion(&mut s, "RETURN 1;");
        assert_eq!(s.history.len(), 2);
        // The trimmed failure still carries its query and error.
        assert_eq!(s.history[0].statement, "BAD");
        assert!(s.history[0].trimmed, "trimmed past the cap");
        assert!(s.history[0].error.is_some(), "error correlation kept");
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

    // --- Yank / clipboard + mouse toggle (issue 04 / ADR 0015) ----------------

    /// A two-column result with one row, focused on the results pane, for yank.
    fn with_two_col_row() -> WorkbenchState {
        let mut s = wb();
        s.focus = Focus::Results;
        s.history.push(CurrentResult {
            header: vec!["name".to_string(), "age".to_string()],
            rows: vec![Record::new(vec![Value::String("Ada".into()), Value::Integer(36)])],
            ..CurrentResult::default()
        });
        s
    }

    #[test]
    fn y_yanks_the_selected_cell_as_an_osc52_copy_effect() {
        let mut s = with_two_col_row();
        // Move the column selection to the second cell, then yank it.
        update(&mut s, Event::Key(Key::plain(KeyCode::Right)));
        let effects = update(&mut s, Event::Key(Key::char('y')));
        assert_eq!(effects, vec![Effect::CopyToClipboard("36".to_string())], "exact cell payload");
        assert!(s.status.message.contains("copied cell"), "status confirms: {:?}", s.status.message);
    }

    #[test]
    fn shift_y_yanks_the_whole_row() {
        let mut s = with_two_col_row();
        let effects = update(&mut s, Event::Key(Key::char('Y')));
        assert_eq!(
            effects,
            vec![Effect::CopyToClipboard("Ada\t36".to_string())],
            "the row's cells, tab-joined as shown"
        );
        assert!(s.status.message.contains("copied row"));
    }

    #[test]
    fn the_cell_detail_overlay_yanks_the_full_value() {
        let mut s = with_two_col_row();
        // Open the cell-detail overlay on the first cell, then yank from it.
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(s.detail.is_some(), "detail overlay open");
        let effects = update(&mut s, Event::Key(Key::char('y')));
        assert_eq!(effects, vec![Effect::CopyToClipboard("Ada".to_string())]);
        assert!(s.status.message.contains("copied value"));
    }

    #[test]
    fn set_mouse_off_then_on_toggles_capture_and_emits_the_effect() {
        let mut s = wb();
        assert!(s.mouse, "capture is on by default");
        let off = submit_meta(&mut s, ":set mouse off");
        assert_eq!(off, vec![Effect::SetMouseCapture(false)]);
        assert!(!s.mouse, "the reducer tracks the toggle");
        assert!(s.status.message.contains("mouse = off"));
        let on = submit_meta(&mut s, ":set mouse on");
        assert_eq!(on, vec![Effect::SetMouseCapture(true)]);
        assert!(s.mouse);
    }

    #[test]
    fn set_lists_the_mouse_setting() {
        let mut s = wb();
        submit_meta(&mut s, ":set");
        assert!(s.status.message.contains("mouse = on"), "mouse listed: {:?}", s.status.message);
    }

    // --- In-result search/filter (issue 16) -----------------------------------

    /// A result with one text column, one cell per given row, focused with a
    /// known viewport — the shape the search tests match against.
    fn with_text_rows(cells: &[&str], viewport: usize) -> WorkbenchState {
        let mut s = wb();
        s.focus = Focus::Results;
        s.viewport_rows = viewport;
        s.history.push(CurrentResult {
            header: vec!["name".to_string()],
            rows: cells
                .iter()
                .map(|c| Record::new(vec![Value::String((*c).to_string())]))
                .collect(),
            ..CurrentResult::default()
        });
        s
    }

    fn open_search_overlay(state: &mut WorkbenchState) {
        update(state, Event::Key(Key::ctrl(KeyCode::Char('f'))));
    }

    #[test]
    fn search_matches_rows_by_substring_across_cells_and_clears_to_the_full_view() {
        let mut s = with_text_rows(&["apple", "banana", "apricot", "cherry"], 10);
        open_search_overlay(&mut s);
        assert!(s.search.is_some(), "search opened");
        type_str(&mut s, "ap");
        let search = s.search.as_ref().expect("open");
        assert_eq!(search.matches, vec![0, 2], "apple and apricot match 'ap'");
        assert_eq!(search.current, Some(0));
        // The selection moved onto the first match.
        assert_eq!(s.shown().unwrap().selected_row, 0);
        // Esc clears search and restores the full view.
        update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(s.search.is_none(), "cleared");
    }

    #[test]
    fn search_is_case_insensitive() {
        let mut s = with_text_rows(&["Apple", "BANANA"], 10);
        open_search_overlay(&mut s);
        type_str(&mut s, "ban");
        assert_eq!(s.search.as_ref().unwrap().matches, vec![1]);
    }

    #[test]
    fn next_and_previous_match_navigation_moves_the_selection() {
        let mut s = with_text_rows(&["apple", "banana", "apricot", "cherry", "grape"], 10);
        open_search_overlay(&mut s);
        type_str(&mut s, "a"); // apple(0), banana(1), apricot(2), grape(4)
        assert_eq!(s.search.as_ref().unwrap().matches, vec![0, 1, 2, 4]);
        assert_eq!(s.shown().unwrap().selected_row, 0);
        // Down steps to the next match.
        update(&mut s, Event::Key(Key::plain(KeyCode::Down)));
        assert_eq!(s.shown().unwrap().selected_row, 1);
        update(&mut s, Event::Key(Key::plain(KeyCode::Down)));
        assert_eq!(s.shown().unwrap().selected_row, 2);
        // Up steps back.
        update(&mut s, Event::Key(Key::plain(KeyCode::Up)));
        assert_eq!(s.shown().unwrap().selected_row, 1);
        // Wrap-around: from the first match, Up lands on the last.
        update(&mut s, Event::Key(Key::plain(KeyCode::Up)));
        update(&mut s, Event::Key(Key::plain(KeyCode::Up)));
        assert_eq!(s.shown().unwrap().selected_row, 4, "wrapped to the last match");
    }

    #[test]
    fn the_filter_toggle_is_reflected_in_state_and_toggles_off() {
        let mut s = with_text_rows(&["apple", "banana", "apricot"], 10);
        open_search_overlay(&mut s);
        type_str(&mut s, "ap");
        // Tab turns on the filtered (matching-rows-only) view.
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert!(s.search.as_ref().unwrap().filter_only, "filter on");
        // Tab again restores the full view (toggles off).
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert!(!s.search.as_ref().unwrap().filter_only, "filter off");
    }

    #[test]
    fn backspace_re_matches_live() {
        let mut s = with_text_rows(&["apple", "apricot", "banana"], 10);
        open_search_overlay(&mut s);
        type_str(&mut s, "app");
        assert_eq!(s.search.as_ref().unwrap().matches, vec![0]);
        update(&mut s, Event::Key(Key::plain(KeyCode::Backspace)));
        // Now "ap" matches both apple and apricot.
        assert_eq!(s.search.as_ref().unwrap().matches, vec![0, 1]);
    }

    #[test]
    fn search_over_a_partial_result_states_loaded_rows_only() {
        let mut s = with_text_rows(&["apple"], 10);
        s.shown_mut().unwrap().truncated = true;
        open_search_overlay(&mut s);
        assert!(s.status.message.contains("loaded rows only"), "{}", s.status.message);
    }

    #[test]
    fn search_with_no_result_is_refused() {
        let mut s = wb();
        s.focus = Focus::Results;
        open_search_overlay(&mut s);
        assert!(s.search.is_none(), "nothing to search");
        assert!(s.status.message.contains("no result"));
    }

    // --- Multiple buffers / tabs (issue 18) -----------------------------------

    fn new_buffer(state: &mut WorkbenchState) {
        update(state, Event::Key(Key::ctrl(KeyCode::Char('t'))));
    }
    fn next_buffer(state: &mut WorkbenchState) {
        update(state, Event::Key(Key::ctrl(KeyCode::PageDown)));
    }
    fn prev_buffer(state: &mut WorkbenchState) {
        update(state, Event::Key(Key::ctrl(KeyCode::PageUp)));
    }
    fn close_buffer(state: &mut WorkbenchState) {
        submit_meta(state, ":close");
    }

    #[test]
    fn ctrl_w_no_longer_closes_a_buffer_and_reaches_the_editor() {
        let mut s = wb();
        new_buffer(&mut s); // two buffers open
        type_str(&mut s, "alpha beta");
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('w'))));
        assert_eq!(s.buffer_count(), 2, "Ctrl+W did not close a buffer");
        // It reached the editor as delete-word.
        assert_eq!(s.editor.buffer(), "alpha ", "Ctrl+W deleted the word in the editor");
    }

    #[test]
    fn new_and_cycle_gestures_manage_buffers_each_with_its_own_editor_text() {
        let mut s = wb();
        type_str(&mut s, "QUERY ONE");
        assert_eq!(s.buffer_count(), 1);

        new_buffer(&mut s);
        assert_eq!(s.buffer_count(), 2);
        assert_eq!(s.active, 1);
        assert_eq!(s.editor.buffer(), "", "a new buffer starts empty");
        type_str(&mut s, "QUERY TWO");

        // Switching back preserves each buffer's own editor text.
        prev_buffer(&mut s);
        assert_eq!(s.active, 0);
        assert_eq!(s.editor.buffer(), "QUERY ONE");
        next_buffer(&mut s);
        assert_eq!(s.active, 1);
        assert_eq!(s.editor.buffer(), "QUERY TWO");
    }

    #[test]
    fn each_buffer_keeps_its_own_result_history() {
        let mut s = wb();
        s.history.push(CurrentResult::new("A".to_string(), vec!["x".to_string()]));
        new_buffer(&mut s);
        assert!(s.history.is_empty(), "the new buffer has its own (empty) history");
        s.history.push(CurrentResult::new("B".to_string(), vec!["y".to_string()]));
        prev_buffer(&mut s);
        assert_eq!(s.history.len(), 1);
        assert_eq!(s.history[0].statement, "A", "buffer 1's result preserved");
        next_buffer(&mut s);
        assert_eq!(s.history[0].statement, "B", "buffer 2's result preserved");
    }

    #[test]
    fn cycling_wraps_around() {
        let mut s = wb();
        new_buffer(&mut s);
        new_buffer(&mut s); // 3 buffers, active 2
        assert_eq!(s.active, 2);
        next_buffer(&mut s);
        assert_eq!(s.active, 0, "wrapped to the first");
        prev_buffer(&mut s);
        assert_eq!(s.active, 2, "wrapped to the last");
    }

    #[test]
    fn closing_the_last_buffer_is_a_no_op() {
        let mut s = wb();
        close_buffer(&mut s);
        assert_eq!(s.buffer_count(), 1, "always at least one buffer");
        assert!(s.status.message.contains("last buffer"));
    }

    #[test]
    fn closing_a_buffer_drops_it_and_activates_a_neighbour() {
        let mut s = wb();
        type_str(&mut s, "ONE");
        new_buffer(&mut s);
        type_str(&mut s, "TWO");
        close_buffer(&mut s); // closes buffer 2, back to buffer 1
        assert_eq!(s.buffer_count(), 1);
        assert_eq!(s.editor.buffer(), "ONE");
    }

    #[test]
    fn session_global_state_is_shared_across_buffers() {
        let mut s = wb();
        // A :param set in one buffer is visible from another (one shared Session).
        s.params.insert("x".to_string(), Value::Integer(1));
        s.read_only = true;
        new_buffer(&mut s);
        assert!(s.params.contains_key("x"), "params shared across buffers");
        assert!(s.read_only, "read-only shared across buffers");
    }

    #[test]
    fn a_second_submit_is_refused_while_a_query_is_live() {
        let mut s = wb();
        submit_query(&mut s, "RETURN 1;");
        // A second submit is still refused (one live result, ADR 0005)…
        type_str(&mut s, "RETURN 2;");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty(), "second submit emits no run effect");
        assert!(s.status.message.contains("busy"));
    }

    #[test]
    fn buffer_navigation_is_allowed_while_a_query_is_live() {
        let mut s = wb();
        submit_query(&mut s, "RETURN 1;");
        // …but opening / switching Buffers is allowed (issue 03): the live query
        // belongs to its origin Buffer and streams there regardless of the view.
        new_buffer(&mut s);
        assert_eq!(s.buffer_count(), 2, "a new buffer opens while a query runs");
        assert_eq!(s.active, 1, "the new buffer is active");
        assert_eq!(s.running_buffer, Some(0), "the query still belongs to buffer 1");
    }

    #[test]
    fn a_live_query_streams_into_its_origin_buffer_not_the_active_one() {
        let mut s = wb();
        let id = submit_query(&mut s, "RETURN 1;");
        // Switch to a fresh buffer before the query even starts.
        new_buffer(&mut s);
        assert_eq!(s.active, 1);
        // The lifecycle events arrive while buffer 2 is active; they must land in
        // buffer 1 (the origin), never the active buffer's history.
        update(&mut s, Event::QueryStarted { id, header: vec!["n".to_string()] });
        update(&mut s, Event::RecordArrived { id, record: one_row() });
        update(
            &mut s,
            Event::QueryCompleted { id, summary: Summary::default(), elapsed: Duration::from_millis(1) },
        );
        assert!(s.history.is_empty(), "the active (background) buffer's history is untouched");
        assert!(matches!(s.run, RunState::Idle), "the query completed");
        assert!(s.running_buffer.is_none(), "the origin buffer is released on idle");
        // Switching back to the origin buffer shows the streamed result.
        prev_buffer(&mut s);
        assert_eq!(s.active, 0);
        assert_eq!(s.history.len(), 1, "buffer 1 holds the completed result");
        assert_eq!(s.history[0].rows.len(), 1, "its row streamed in while it was a background buffer");
    }

    #[test]
    fn a_tab_click_is_allowed_while_a_query_is_live() {
        let mut s = wb();
        new_buffer(&mut s); // two buffers; active is buffer 2
        let id = submit_query(&mut s, "RETURN 1;");
        assert_eq!(s.running_buffer, Some(1));
        s.tabbar_area = Rect::new(0, 0, super::super::draw::TAB_WIDTH * 2, 1);
        click(&mut s, 1, 0); // click buffer 1's tab while buffer 2's query runs
        assert_eq!(s.active, 0, "switched to buffer 1 mid-query");
        // The query's records still route to buffer 2.
        update(&mut s, Event::QueryStarted { id, header: vec!["n".to_string()] });
        update(&mut s, Event::RecordArrived { id, record: one_row() });
        assert!(s.history.is_empty(), "buffer 1 (now active) is untouched by buffer 2's query");
        next_buffer(&mut s);
        assert_eq!(s.history.len(), 1, "buffer 2 received its result");
    }

    #[test]
    fn a_tab_bar_click_switches_buffers() {
        let mut s = wb();
        type_str(&mut s, "ONE");
        new_buffer(&mut s); // buffer 2 active
        // The draw would set tabbar_area; simulate a 2-tab bar at the top.
        s.tabbar_area = Rect::new(0, 0, super::super::draw::TAB_WIDTH * 2, 1);
        // Click the first tab (columns 0..TAB_WIDTH).
        click(&mut s, 1, 0);
        assert_eq!(s.active, 0, "clicked the first tab");
        assert_eq!(s.editor.buffer(), "ONE");
    }

    // --- Mouse support (issue 17) ---------------------------------------------

    use super::super::event::{MouseEvent, MouseKind};
    use ratatui::layout::Rect;

    /// A result with known editor/results rectangles cached, as the draw would set
    /// them, so the mouse hit-tests have geometry to work with.
    fn with_mouse_layout(cells: &[&str]) -> WorkbenchState {
        let mut s = with_text_rows(cells, 10);
        s.focus = Focus::Editor;
        s.editor_area = Rect::new(0, 0, 40, 5);
        // Results sit below the editor; header at row 6, data rows from row 7.
        s.results_area = Rect::new(0, 6, 40, 8);
        s
    }

    fn click(state: &mut WorkbenchState, column: u16, row: u16) {
        update(state, Event::Mouse(MouseEvent { kind: MouseKind::Down, column, row }));
    }

    fn scroll(state: &mut WorkbenchState, kind: MouseKind, column: u16, row: u16) {
        update(state, Event::Mouse(MouseEvent { kind, column, row }));
    }

    #[test]
    fn clicking_a_pane_focuses_it() {
        let mut s = with_mouse_layout(&["apple", "banana"]);
        // Click in the results rect focuses Results.
        click(&mut s, 1, 6);
        assert_eq!(s.focus, Focus::Results);
        // Click back in the editor rect focuses Editor.
        click(&mut s, 1, 1);
        assert_eq!(s.focus, Focus::Editor);
    }

    #[test]
    fn clicking_a_result_cell_selects_it_and_opens_cell_detail() {
        let mut s = with_mouse_layout(&["apple", "banana", "cherry"]);
        // results_area.y = 6 (header), so data row 1 is at terminal row 8.
        click(&mut s, 3, 8);
        assert_eq!(s.focus, Focus::Results);
        assert_eq!(s.shown().unwrap().selected_row, 1, "second data row selected");
        assert!(s.detail.is_some(), "the cell-expand overlay opened");
    }

    #[test]
    fn clicking_the_header_row_only_focuses_without_selecting() {
        let mut s = with_mouse_layout(&["apple", "banana"]);
        click(&mut s, 1, 6); // the header row
        assert_eq!(s.focus, Focus::Results);
        assert!(s.detail.is_none(), "no cell expanded from a header click");
    }

    #[test]
    fn the_scroll_wheel_moves_through_result_rows() {
        let mut s = with_mouse_layout(&["a", "b", "c", "d"]);
        s.focus = Focus::Results;
        scroll(&mut s, MouseKind::ScrollDown, 1, 8);
        assert_eq!(s.shown().unwrap().selected_row, 1);
        scroll(&mut s, MouseKind::ScrollDown, 1, 8);
        assert_eq!(s.shown().unwrap().selected_row, 2);
        scroll(&mut s, MouseKind::ScrollUp, 1, 8);
        assert_eq!(s.shown().unwrap().selected_row, 1);
    }

    #[test]
    fn the_scroll_wheel_moves_the_editor_viewport() {
        let mut s = with_mouse_layout(&["a"]);
        s.editor.set_text("line one\nline two");
        // Put the cursor on the first line deterministically.
        s.editor.edit(Key::plain(KeyCode::Up));
        let before = s.editor.cursor().0;
        scroll(&mut s, MouseKind::ScrollDown, 1, 1); // over the editor rect
        assert_eq!(
            s.editor.cursor().0,
            before + 1,
            "scroll moved the editor cursor down a line"
        );
    }

    #[test]
    fn the_mouse_is_ignored_while_an_overlay_is_open() {
        let mut s = with_mouse_layout(&["apple", "banana"]);
        s.detail = Some(Value::Integer(1));
        click(&mut s, 1, 8);
        // Focus unchanged and no new selection — the overlay owns input.
        assert_eq!(s.focus, Focus::Editor);
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

    // --- :help keybinding overlay ---------------------------------------------

    #[test]
    fn help_opens_a_dismissable_overlay() {
        let mut s = wb();
        submit_meta(&mut s, ":help");
        assert!(s.help, "the help overlay opened");
        assert_eq!(s.editor.buffer(), "", "the command was consumed");
        // While open, Down scrolls and Esc closes (the overlay owns input).
        update(&mut s, Event::Key(Key::plain(KeyCode::Down)));
        assert_eq!(s.help_scroll, 1);
        update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(!s.help, "Esc dismisses the overlay rather than quitting");
    }

    #[test]
    fn the_help_text_lists_gestures_with_their_live_chords() {
        // Default bindings show through; a [keys] rebinding is reflected, not the
        // default — the help reads the live KeyBindings.
        let help = keybindings_help(&KeyBindings::default());
        assert!(help.contains("New tab"), "buffers section present: {help}");
        assert!(help.contains("ctrl+t"), "default new-buffer chord shown");
        assert!(help.contains("Search / filter"), "search listed");
        assert!(help.contains(":save"), "commands referenced");

        let (keys, _) = crate::theme::resolve_keys(&BTreeMap::from([(
            "new-buffer".to_string(),
            "ctrl+n".to_string(),
        )]));
        let rebound = keybindings_help(&keys);
        assert!(rebound.contains("ctrl+n"), "the rebound chord is shown: {rebound}");
        assert!(!rebound.contains("ctrl+t"), "the old default chord is gone");
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

    // --- Auto-format gesture (issue 15) ---------------------------------------

    #[test]
    fn the_format_gesture_reformats_the_editor_buffer() {
        let mut s = wb();
        type_str(&mut s, "match (n) return n");
        // Ctrl+L is the default format-buffer chord (issue 02).
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('l'))));
        assert_eq!(s.editor.buffer(), "MATCH (n)\nRETURN n");
        assert_eq!(s.status.message, "formatted");
    }

    #[test]
    fn the_format_gesture_leaves_partial_input_untouched_with_a_status() {
        let mut s = wb();
        type_str(&mut s, "RETURN 'half");
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('l'))));
        assert_eq!(s.editor.buffer(), "RETURN 'half", "untouched");
        assert!(s.status.message.contains("unterminated string"), "{}", s.status.message);
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
