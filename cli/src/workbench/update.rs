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
use crate::syntax::{word_start, Completer};
use crate::theme::{Chord, ChordKey, Gesture, KeyBindings};

use super::effect::{Effect, TxOp};
use super::event::{Event, Key, KeyCode, MouseEvent, MouseKind};
use super::plan::{is_plan_query, Plan};
use super::schema::{Schema, SchemaSource};
use super::state::{
    CommandCompletion, CommandLine, Completion, CurrentResult, Disposition, DrawerKind,
    ExportPrompt, Focus, Modal, ModalKind, RunState, SearchState, TransactionTag, WatchState,
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
            on_transaction_applied(state, tx, message);
            Vec::new()
        }
        Event::Connected(result) => {
            match result {
                Ok(connected) => {
                    // The swap aborts any open transaction (ADR 0011): resolve its
                    // episode's entries as rolled back before the marker resets, so
                    // they are not left dangling `[open]` (issue 04).
                    if state.tx != mgconsole_core::TransactionState::Auto {
                        resolve_episode(state, Disposition::RolledBack);
                    }
                    state.pending_tx_op = None;
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
    state.run.running_id() == Some(id)
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
    // Mark the in-flight statement started, and take its statement text + tag for
    // the entry. `run` is `Running` here (is_current passed).
    let (statement, tag) = if let RunState::Running { started, statement, tag, .. } = &mut state.run {
        *started = true;
        (statement.clone(), *tag)
    } else {
        return;
    };
    let cap = state.config.history_cap;
    if let Some((history, view)) = state.running_target() {
        let mut result = CurrentResult::new(statement, header);
        result.tx_tag = tag;
        history.push(result);
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
    // `run` is `Running` here (is_current passed): take the statement, started flag,
    // and tag for the entry.
    let (statement, started, tag) = if let RunState::Running { statement, started, tag, .. } = &state.run {
        (statement.clone(), *started, *tag)
    } else {
        return Vec::new();
    };
    let cap = state.config.history_cap;
    if let Some((history, view)) = state.running_target() {
        if started {
            if let Some(result) = history.last_mut() {
                result.error = Some(error_text.clone());
            }
        } else {
            let mut result = CurrentResult::new(statement, Vec::new());
            result.error = Some(error_text.clone());
            result.tx_tag = tag;
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
    state.spinner = 0;
    // Compute the Transaction tag first — it advances `tx_ordinal` (issue 04) — then
    // construct the in-flight statement as one value.
    let tag = next_transaction_tag(state);
    state.run = RunState::Running {
        id,
        statement: query.clone(),
        started: false,
        tag,
    };
    if state.running_buffer.is_none() {
        state.running_buffer = Some(state.active);
    }
    state.status.message = "running…".to_string();
    // Remember the last query so `:watch` with no query can reuse it (issue 11).
    state.last_query = Some(query.clone());
    vec![Effect::RunQuery {
        id,
        query,
        params: state.params.clone(),
    }]
}

/// The Transaction tag for the statement about to run (issue 04): when a
/// transaction is open, advance the episode's per-statement ordinal and tag the
/// statement with the episode number, that ordinal, and `Open` (resolved later).
/// An autocommit statement carries no tag. Ordinals continue across Buffers because
/// the counters are session-scoped on the state.
fn next_transaction_tag(state: &mut WorkbenchState) -> Option<TransactionTag> {
    if state.tx == mgconsole_core::TransactionState::Auto {
        return None;
    }
    state.tx_ordinal += 1;
    Some(TransactionTag {
        episode: state.tx_episode,
        ordinal: state.tx_ordinal,
        disposition: Disposition::Open,
    })
}

/// Apply a `TransactionApplied` event (issue 04/05): mirror the Session's
/// transaction state, show any message, and — driven by the op that was awaiting
/// its result — open a new episode (`:begin` → Open) or resolve the current one's
/// entries across every Buffer (`:commit` → committed, `:rollback` → rolled back).
/// A query-driven poison sync (no pending op) only updates the marker; the poisoned
/// episode stays Open until a later `:rollback` resolves it.
fn on_transaction_applied(
    state: &mut WorkbenchState,
    tx: mgconsole_core::TransactionState,
    message: Option<String>,
) {
    use mgconsole_core::TransactionState;
    state.tx = tx;
    if let Some(message) = message {
        state.status.message = message;
    }
    let Some(op) = state.pending_tx_op.take() else {
        return;
    };
    match op {
        // A new episode opened: bump the number and restart the ordinal sequence.
        TxOp::Begin if tx == TransactionState::Open => {
            state.tx_episode += 1;
            state.tx_ordinal = 0;
        }
        // The episode ended: flip every still-open entry of it, in all Buffers.
        TxOp::Commit if tx == TransactionState::Auto => {
            resolve_episode(state, Disposition::Committed);
        }
        TxOp::Rollback if tx == TransactionState::Auto => {
            resolve_episode(state, Disposition::RolledBack);
        }
        // The op did not take effect (e.g. a poisoned `:commit` the server rejected,
        // which leaves the transaction Failed): leave the episode unresolved.
        _ => {}
    }
}

/// Resolve the current Transaction episode's entries to `disposition` (issue 04),
/// retroactively across every Buffer's Result history: each entry tagged with the
/// current episode and still `Open` flips. A rolled-back entry keeps its rows (it
/// stays navigable as a record of what ran) but now reads as undone.
fn resolve_episode(state: &mut WorkbenchState, disposition: Disposition) {
    fn resolve_in(history: &mut [CurrentResult], episode: u32, disposition: Disposition) {
        for result in history {
            if let Some(tag) = result.tx_tag.as_mut() {
                if tag.episode == episode && tag.disposition == Disposition::Open {
                    tag.disposition = disposition;
                }
            }
        }
    }
    let episode = state.tx_episode;
    // The active Buffer's live history, then every parked Buffer's (issue 03/18):
    // all entries of the ending episode resolve together, wherever they were run.
    resolve_in(&mut state.history, episode, disposition);
    for buffer in &mut state.buffers {
        resolve_in(&mut buffer.history, episode, disposition);
    }
}

/// Ctrl-C. While a query runs, cancel it: keep the rows already streamed on
/// screen but label the result partial, drop the rest of the batch, fall idle,
/// and emit [`Effect::Cancel`] (the edge aborts the task and RESETs the Session,
/// ADR 0005). When idle, abandon the typed buffer (the REPL's interrupt).
fn interrupt(state: &mut WorkbenchState) -> Vec<Effect> {
    if let Some(id) = state.run.running_id() {
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
    // The one open Modal captures keys while open — including Esc, so it dismisses
    // the overlay rather than quitting (ADR 0017). One `match` on the open overlay,
    // not six `is_some()` checks: search edits its query and steps matches; the
    // completion popup cycles candidates; the Command line runs the typed
    // `:`-vocabulary; the `?` help overlay scrolls.
    if let Some(kind) = state.modal_kind() {
        return match kind {
            ModalKind::Help => help_key(state, key),
            ModalKind::Detail => detail_key(state, key),
            ModalKind::Export => export_key(state, key),
            ModalKind::Search => search_key(state, key),
            ModalKind::Completion => completion_key(state, key),
            ModalKind::CommandLine => command_line_key(state, key),
        };
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
    // Editor undo/redo (issue 09): Ctrl+Z reverts the last edit — including a
    // whole-buffer replace from auto-format / `:load` / history recall — and Ctrl+Y
    // redoes. These are editor-owned chords (ADR 0016); the summary drawer moved off
    // Ctrl+Y to Ctrl+N, so they bind globally with no collision.
    if key.ctrl && key.code == KeyCode::Char('z') {
        state.editor.undo();
        return Vec::new();
    }
    if key.ctrl && key.code == KeyCode::Char('y') {
        state.editor.redo();
        return Vec::new();
    }
    // Bare `?` opens the help overlay when the editor is empty or the results pane
    // is focused (issue 10); typing `?` into a non-empty query still inserts it, so
    // a `?` in Cypher text is never swallowed.
    if key.code == KeyCode::Char('?') && !key.ctrl && !key.alt {
        let editor_empty = state.editor.buffer().is_empty();
        if matches!(state.focus, Focus::Results) || editor_empty {
            state.modal = Some(Modal::Help { scroll: 0 });
            return Vec::new();
        }
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
/// mouse state. Hit-testing uses the pane rectangles the draw cached. While any
/// [`Modal`] is open the mouse is ignored, so it never fights the keyboard-driven
/// overlay or opens a second one on top of it.
fn update_mouse(state: &mut WorkbenchState, mouse: MouseEvent) -> Vec<Effect> {
    // While any Modal is open the mouse is ignored, so it never opens a second
    // overlay on top of a keyboard-driven one (the one field covers all six —
    // previously this list omitted search/command-line, so a click while searching
    // could open the cell-detail overlay over it).
    if state.modal.is_some() {
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
    // Map the click to a Buffer through the windowed layout the draw cached (issue
    // 07): each visible tab owns the column range `[col, col + width)`. A click on
    // an overflow marker or empty space hits no tab and is ignored.
    let hit = state
        .tab_hits
        .iter()
        .find(|tab| col >= tab.col && col < tab.col + tab.width)
        .map(|tab| tab.index);
    if let Some(index) = hit {
        if index != state.active {
            state.switch_to(index);
            state.status.message = format!("buffer {}/{}", state.active + 1, state.buffer_count());
        }
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
    if state.search().is_some_and(|s| s.filter_only) {
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
        // Open the modal Command line mid-composition (ADR 0017): the editor's
        // half-written Cypher is left untouched, and focus returns to it on
        // Esc/Enter. The prompt is seeded with `:` so a command is just typed.
        Gesture::OpenCommandLine => {
            open_command_line(state);
            Vec::new()
        }
        // Shift+Tab with the completion popup closed switches focus between the
        // editor and the results pane (ADR 0017); with the popup open, Shift+Tab is
        // intercepted earlier as prev-candidate, so it never reaches here.
        Gesture::SwitchFocus => {
            state.focus = match state.focus {
                Focus::Editor => Focus::Results,
                Focus::Results => Focus::Editor,
            };
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
    state.modal = Some(Modal::Search(SearchState::default()));
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
            state.modal = None;
            state.status.message = "search cleared".to_string();
        }
        Key { code: KeyCode::Backspace, .. } => {
            if let Some(search) = state.search_mut() {
                search.query.pop();
            }
            recompute_search(state);
        }
        // Tab toggles the filtered view (only matching rows) on/off.
        Key { code: KeyCode::Tab, .. } => {
            if let Some(search) = state.search_mut() {
                search.filter_only = !search.filter_only;
            }
            update_search_status(state);
        }
        // Enter / Down step to the next match; Up to the previous (wrapping).
        Key { code: KeyCode::Enter | KeyCode::Down, .. } => search_step(state, 1),
        Key { code: KeyCode::Up, .. } => search_step(state, -1),
        // Ordinary printable input extends the query and re-matches live.
        Key { code: KeyCode::Char(c), ctrl: false, alt: false, .. } => {
            if let Some(search) = state.search_mut() {
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
    let needle = state.search()
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
    if let Some(search) = state.search_mut() {
        search.current = (!matches.is_empty()).then_some(0);
        search.matches = matches;
    }
    move_selection_to_match(state);
    update_search_status(state);
}

/// Step the active match by `delta` (wrapping) and move the selection onto it.
fn search_step(state: &mut WorkbenchState, delta: isize) {
    if let Some(search) = state.search_mut() {
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
    let target = state.search()
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
    let Some(search) = state.search() else {
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
    // Shift+Tab cycles to the previous candidate (ADR 0017), the mirror of Tab's
    // next; checked first since it shares the Tab key code.
    if key.code == KeyCode::Tab && key.shift {
        cycle_completion(state, -1);
        return Vec::new();
    }
    match key.code {
        KeyCode::Esc => {
            state.modal = None;
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
            state.modal = None;
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
        // Tab is completion-only (ADR 0017): it opens the popup for the word under
        // the cursor. With nothing to complete (empty prefix or no candidates) it is
        // a no-op — it never switches pane focus. Focus-switch is Shift+Tab, the
        // SwitchFocus gesture. (Shift+Tab is caught by the gesture layer before
        // reaching here, so this arm only sees plain Tab.)
        Key {
            code: KeyCode::Tab,
            ctrl: false,
            alt: false,
            shift: false,
        } => {
            open_completion(state);
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
        // `:` on an empty/whitespace-only editor opens the modal Command line with
        // `:` already present (ADR 0017), mirroring the REPL's "line starts with `:`"
        // rule. Once any Cypher is present, `:` is literal text (labels, rel-types,
        // map keys, enums), so it is never swallowed mid-query.
        Key {
            code: KeyCode::Char(':'),
            ctrl: false,
            alt: false,
            ..
        } if state.editor.buffer().trim().is_empty() => {
            open_command_line(state);
            Vec::new()
        }
        // Everything else is ordinary editing, delegated to the editor widget.
        other => {
            state.editor.edit(other);
            Vec::new()
        }
    }
}

/// Open the modal Command line (ADR 0017), seeded with `:` and remembering the
/// current focus to restore when it closes. The editor buffer is never touched —
/// the command line is a separate surface, so a half-written query is preserved.
fn open_command_line(state: &mut WorkbenchState) {
    state.modal = Some(Modal::CommandLine(CommandLine::new(state.focus)));
}

/// Keys while the modal Command line is open (ADR 0017 / issue 02): Enter runs the
/// typed `:`-command, Esc cancels, Tab completes a `:command` name, Up/Down recall
/// past commands, Backspace/printable input edit the line. Closing it (either way)
/// restores the focus the prompt was opened from — the command line is not part of
/// the focus cycle, so Tab here never switches panes.
fn command_line_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    match key.code {
        KeyCode::Enter => run_command_line(state),
        KeyCode::Esc => {
            close_command_line(state);
            Vec::new()
        }
        // Tab completes the `:command` name under the cursor from the existing
        // completion engine (issue 02); with no candidates it is a no-op.
        KeyCode::Tab => {
            complete_command_line(state);
            Vec::new()
        }
        // Up/Down walk the session command history — a stack distinct from the
        // Cypher query history that Ctrl+Up/Down recalls into the editor (issue 02).
        KeyCode::Up => {
            recall_command(state, true);
            Vec::new()
        }
        KeyCode::Down => {
            recall_command(state, false);
            Vec::new()
        }
        KeyCode::Backspace => {
            if let Some(cl) = state.command_line_mut() {
                cl.content.pop();
                cl.completion = None;
            }
            Vec::new()
        }
        KeyCode::Char(c) if !key.ctrl && !key.alt => {
            if let Some(cl) = state.command_line_mut() {
                cl.content.push(c);
                cl.completion = None;
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Complete the `:command` name under the cursor in the Command line (issue 02). A
/// repeated Tab cycles the open menu; a fresh Tab computes candidates for the word
/// and applies the first, replacing the typed prefix. With no candidates it is a
/// no-op (and never switches focus — the command line is modal).
fn complete_command_line(state: &mut WorkbenchState) {
    let Some(cl) = state.command_line_mut() else {
        return;
    };
    // A repeated Tab cycles the already-open menu rather than re-deriving it (which
    // would collapse to the single just-inserted candidate).
    if let Some(menu) = cl.completion.as_mut() {
        menu.selected = (menu.selected + 1) % menu.candidates.len();
        cl.content = format!("{}{}", menu.base, menu.candidates[menu.selected]);
        return;
    }
    let start = word_start(&cl.content, cl.content.len());
    let word = cl.content[start..].to_string();
    let candidates = Completer::with_command_vocabulary().candidates(&word);
    if candidates.is_empty() {
        return;
    }
    let base = cl.content[..start].to_string();
    cl.content = format!("{base}{}", candidates[0]);
    cl.completion = Some(CommandCompletion {
        candidates,
        selected: 0,
        base,
    });
}

/// Recall an older (`older`) or newer past command into the Command line (issue
/// 02), over the session command history. On the first older step the in-progress
/// line is saved; stepping past the newest restores it. A no-op when there is
/// nothing to recall in that direction.
fn recall_command(state: &mut WorkbenchState, older: bool) {
    let history = &state.command_history;
    let len = history.len();
    // Inline the pattern (not `command_line_mut()`): this borrows only `state.modal`,
    // disjoint from `state.command_history` borrowed above as `history`.
    let Some(Modal::CommandLine(cl)) = &mut state.modal else {
        return;
    };
    cl.completion = None;
    if older {
        if len == 0 {
            return;
        }
        let index = match cl.recall_index {
            None => {
                cl.recall_saved = Some(cl.content.clone());
                len - 1
            }
            Some(i) => i.saturating_sub(1),
        };
        cl.recall_index = Some(index);
        cl.content.clone_from(&history[index]);
    } else {
        let Some(index) = cl.recall_index else {
            return;
        };
        if index + 1 < len {
            cl.recall_index = Some(index + 1);
            cl.content.clone_from(&history[index + 1]);
        } else {
            cl.recall_index = None;
            cl.content = cl.recall_saved.take().unwrap_or_else(|| ":".to_string());
        }
    }
}

/// Close the Command line, restoring the focus it was opened from.
fn close_command_line(state: &mut WorkbenchState) {
    if let Some(Modal::CommandLine(cl)) = state.modal.take() {
        state.focus = cl.prior_focus;
    }
}

/// Run the typed Command line content (ADR 0017): route it through the Workbench's
/// own command layer (`:close`) and then the shared `MetaCommand` vocabulary, the
/// same dispatch a submitted line used before the editor became Cypher-only. A
/// bare `:` (or empty line) is a silent cancel. Focus returns to where it was.
fn run_command_line(state: &mut WorkbenchState) -> Vec<Effect> {
    let Some(Modal::CommandLine(cl)) = state.modal.take() else {
        return Vec::new();
    };
    state.focus = cl.prior_focus;
    let line = cl.content.trim();
    // A bare prompt (`:` with nothing after it) cancels, like an empty submit.
    if line.is_empty() || line == ":" {
        return Vec::new();
    }
    // Record into the session command history for recall (issue 02), skipping a
    // consecutive duplicate.
    if state.command_history.last().map(String::as_str) != Some(line) {
        state.command_history.push(line.to_string());
    }
    if let Some(command) = workbench_command(line) {
        return handle_workbench_command(state, command);
    }
    // A non-`:` line cannot normally reach here (the prompt always carries `:`),
    // but guard anyway so an edited-away colon reports rather than runs as Cypher.
    if let Some(meta) = meta_command(line) {
        handle_meta(state, meta)
    } else {
        state.status.message = format!("not a command: {line}");
        Vec::new()
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
    state.modal = Some(Modal::Completion(Completion {
        candidates,
        selected: 0,
        prefix_len: prefix.chars().count(),
    }));
}

/// Move the completion selection by `delta`, wrapping around the candidate list.
fn cycle_completion(state: &mut WorkbenchState, delta: isize) {
    if let Some(completion) = state.completion_mut() {
        let len = completion.candidates.len() as isize;
        completion.selected = (completion.selected as isize + delta).rem_euclid(len) as usize;
    }
}

/// Insert the selected candidate, replacing the typed prefix, and close the popup.
fn apply_completion(state: &mut WorkbenchState) {
    if let Some(Modal::Completion(completion)) = state.modal.take() {
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
        // Tab no longer switches focus here (ADR 0017): focus-switch is Shift+Tab
        // (the SwitchFocus gesture, handled before this). Plain Tab is a no-op.
        KeyCode::Tab => return Vec::new(),
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
            state.modal = Some(Modal::Detail { value: Value::String(statement), scroll: 0 });
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
        state.modal = Some(Modal::Detail { value, scroll: 0 });
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
        state.modal = Some(Modal::Export(ExportPrompt::default()));
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
            state.modal = None;
            return Vec::new();
        }
        _ => {}
    }
    if let Some(Modal::Export(prompt)) = &mut state.modal {
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
    let Some(Modal::Export(prompt)) = &state.modal else {
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
    state.modal = None;
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
    let Some(Modal::Help { scroll }) = &mut state.modal else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Up | KeyCode::PageUp => *scroll = scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::PageDown => *scroll = scroll.saturating_add(1),
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => state.modal = None,
        _ => {}
    }
    Vec::new()
}

/// Keys while the cell-detail overlay is open: scroll it, or dismiss it back to
/// the table (with the table selection intact, since it was never changed).
fn detail_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    let Some(Modal::Detail { value, scroll }) = &mut state.modal else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Up | KeyCode::PageUp => *scroll = scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::PageDown => *scroll = scroll.saturating_add(1),
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => state.modal = None,
        // The cell-detail overlay yanks the same way as the results pane (issue
        // 04 / ADR 0015): 'y' copies the full Value shown via OSC 52.
        KeyCode::Char('y') => {
            let text = mgconsole_core::render::tabular(value);
            state.status.message = "copied value".to_string();
            return vec![Effect::CopyToClipboard(text)];
        }
        _ => {}
    }
    Vec::new()
}

/// A Workbench command (CONTEXT.md): a typed `:`-command driving a Workbench-only
/// concept, distinct from the cross-frontend [`MetaCommand`] vocabulary. Parsed by
/// the Workbench's own layer, so the line REPL never recognises it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkbenchCommand {
    /// `:close` — close the active Buffer (the typed, deliberate counterpart to the
    /// navigation gestures, used because closing is destructive).
    Close,
}

/// Recognise a Workbench command in a submitted `:`-line, or `None` if it is not
/// one (a shared meta-command or ordinary query text). Trailing arguments are
/// ignored — `:close` takes none.
fn workbench_command(line: &str) -> Option<WorkbenchCommand> {
    let keyword = line.trim().strip_prefix(':')?.split_whitespace().next()?;
    match keyword {
        "close" => Some(WorkbenchCommand::Close),
        _ => None,
    }
}

/// Handle a Workbench command. `:close` closes the active Buffer; the last Buffer
/// always stays open. Closing is destructive by design, so it always proceeds —
/// if the active Buffer owns the in-flight query, that query is cancelled as part
/// of closing (the buffer and its streaming result are dropped together).
fn handle_workbench_command(state: &mut WorkbenchState, command: WorkbenchCommand) -> Vec<Effect> {
    match command {
        WorkbenchCommand::Close => {
            // Closing swaps the neighbour's state into the live fields (or, for the
            // last Buffer, leaves it in place), so the editor is never cleared here —
            // a `:close` from the Command line must not wipe a Buffer's Cypher.
            if state.buffer_count() == 1 {
                state.status.message = "the last buffer stays open".to_string();
                return Vec::new();
            }
            // If the buffer being closed owns the live query, cancel it: its result
            // streams into this buffer's history, which is about to be dropped.
            let mut effects = Vec::new();
            if state.running_buffer == Some(state.active) {
                if let Some(id) = state.run.running_id() {
                    effects.push(Effect::Cancel { id });
                }
                state.run = RunState::Idle;
                state.running_buffer = None;
                state.pending.clear();
            }
            state.close_buffer();
            state.status.message =
                format!("buffer {}/{}", state.active + 1, state.buffer_count());
            effects
        }
    }
}

/// Handle a submit (plain Enter): split the buffer into statements and run the
/// first, queuing the rest to run sequentially on the one Session. The editor is
/// Cypher-only (ADR 0017), so a `:`-line typed here is *not* a command — it is
/// Cypher and will error; the typed `:`-vocabulary comes from the modal Command
/// line (`Ctrl+G` / `:` on an empty editor). Submitting while a query is in flight
/// is refused with a "session busy" status (one-live-result, ADR 0005). The editor
/// keeps its text so the query can be edited and re-run.
fn submit(state: &mut WorkbenchState) -> Vec<Effect> {
    let buffer = state.editor.buffer();
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
    state.spinner = 0;
    // A `:o` redirect produces no Result-history entry (its result streams to a
    // file), so it carries no Transaction tag and does not advance the ordinal.
    state.run = RunState::Running {
        id,
        statement: query.clone(),
        started: false,
        tag: None,
    };
    if state.running_buffer.is_none() {
        state.running_buffer = Some(state.active);
    }
    state.status.message = format!("running… → {} ({format})", path.display());
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

/// Handle a `:`-meta command run from the modal Command line (ADR 0017), reusing
/// the REPL's `MetaCommand`. The `:param` family (slice 16) reuses the REPL's store
/// and server-side evaluation. The editor is *not* touched — it holds the Buffer's
/// Cypher, a surface separate from the command line — except by `:load`, whose
/// whole purpose is to recall a template into it.
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
            vec![Effect::EvaluateParam {
                name,
                expr,
                params: state.params.clone(),
            }]
        }
        MetaCommand::ListParams => {
            state.drawer = Some(DrawerKind::Params);
            Vec::new()
        }
        MetaCommand::ClearParams => {
            state.params.clear();
            state.status.message = "parameters cleared".to_string();
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
            Vec::new()
        }
        // `mouse` (issue 04) is a Workbench-only session toggle, not a precedence-
        // resolved Setting: `:set mouse off` releases mouse capture so native
        // selection works; `:set mouse on` restores the Workbench gestures.
        MetaCommand::SetSetting { name, value } if name == "mouse" => {
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
            match crate::repl::parse_on_off(&value) {
                Ok(true) => {
                    state.read_only = true;
                    state.status.message = "readonly = on".to_string();
                    return vec![Effect::SetReadOnly(true)];
                }
                Ok(false) => {
                    state.status.message =
                        format!("error: {}", crate::repl::READONLY_OFF_AT_RUNTIME);
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
            let op = match meta {
                MetaCommand::Begin => TxOp::Begin,
                MetaCommand::Commit => TxOp::Commit,
                _ => TxOp::Rollback,
            };
            // Remember the op so its TransactionApplied result can open or resolve
            // the episode by the actual outcome (issue 04).
            state.pending_tx_op = Some(op);
            vec![Effect::Transaction(op)]
        }
        // `:connect` swaps the whole Session (issue 07). Refuse mid-query; warn
        // that an open transaction is aborted by the swap (ADR 0011).
        MetaCommand::Connect(target) => {
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
                return Vec::new();
            }
            if state.tx != mgconsole_core::TransactionState::Auto {
                state.status.message = format!("note: {}", crate::repl::TX_ABORTED_BY_CONNECT);
            }
            vec![Effect::Connect(target)]
        }
        // `:use` switches the active Database (issue 08); refuse mid-query.
        MetaCommand::Use(database) => {
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
                return Vec::new();
            }
            vec![Effect::UseDatabase(database)]
        }
        // `:o` arms the next submitted query to stream to a file (issue 12);
        // one-shot. An unknown format/extension is reported.
        MetaCommand::Redirect(args) => {
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
            vec![Effect::Source(PathBuf::from(path))]
        }
        // `:watch` re-runs a query on a timer (issue 11), refused while a
        // transaction is open. The first run starts immediately; the tick timer
        // drives the rest, each replacing the previous snapshot. Any key stops it.
        MetaCommand::Watch(args) => {
            if state.tx != mgconsole_core::TransactionState::Auto {
                state.status.message = format!("error: {}", crate::repl::WATCH_REFUSED_IN_TX);
                return Vec::new();
            }
            if matches!(state.run, RunState::Running { .. }) {
                state.status.message = "session busy — cancel first".to_string();
                return Vec::new();
            }
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
            match query.or_else(|| state.last_query.clone()) {
                Some(text) => {
                    state.queries.set(name.clone(), text);
                    state.status.message = format!("saved '{name}'");
                    return vec![Effect::PersistQuery(name)];
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
            if state.queries.remove(&name) {
                state.status.message = format!("forgot '{name}'");
                return vec![Effect::PersistQuery(name)];
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
        // `:help`/`:docs` open the keybinding + command overlay (its chords reflect
        // the live `[keys]` bindings). Dismissed by Esc/Enter/q.
        MetaCommand::Help | MetaCommand::Docs => {
            state.modal = Some(Modal::Help { scroll: 0 });
            Vec::new()
        }
    }
}

/// The status-bar keybind hint (issue 10), generated from the live `[keys]` so it
/// shows the user's actual chords (e.g. a rebound format key) rather than a stale
/// fixed string, and points at the help overlay (`? help`). The structural keys
/// (Enter/newline/Tab/Ctrl-C/quit) are fixed and not rebindable.
pub fn status_hint(keys: &KeyBindings, newline_hint: &str) -> String {
    format!(
        "Enter: run · {newline}: newline · Tab: complete · Shift+Tab: focus · {fmt}: format · ? help · Ctrl-C: cancel · Esc/Ctrl-D: quit",
        newline = newline_hint,
        fmt = keys.chord(Gesture::FormatBuffer),
    )
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
            (
                format!(": / {}", chord(Gesture::OpenCommandLine)),
                "Open the command line (run a :command)",
            ),
            ("Tab".to_string(), "Complete the word under the cursor"),
            ("Shift+Tab".to_string(), "Previous candidate, or switch pane focus"),
            ("Ctrl+Z / Ctrl+Y".to_string(), "Undo / redo (incl. format, :load, recall)"),
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
        "Commands (type at the command line — : on an empty editor, or Ctrl+G)\n  \
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

    /// Run a `:`-command through the modal Command line (ADR 0017): open the
    /// command line (`Ctrl+G` seeds the `:` prompt), type the rest, and press
    /// Enter — the path the editor no longer takes. Returns the effects.
    fn run_command(state: &mut WorkbenchState, command: &str) -> Vec<Effect> {
        update(state, Event::Key(Key::ctrl(KeyCode::Char('g'))));
        let rest = command.strip_prefix(':').unwrap_or(command);
        type_str(state, rest);
        update(state, Event::Key(Key::plain(KeyCode::Enter)))
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
        assert!(matches!(s.run, RunState::Running { id: 0, .. }));
        assert_eq!(s.editor.buffer(), "RETURN 1;", "the buffer is kept for re-run");
    }

    #[test]
    fn the_in_flight_statement_lives_inside_the_running_variant() {
        // The per-statement facts are carried in `Running`, not dangling fields, so
        // they cannot outlive the query; `running_id()` is the id-only accessor.
        let mut s = wb();
        type_str(&mut s, "RETURN 1");
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        match &s.run {
            RunState::Running { id, statement, started, tag } => {
                assert_eq!(*id, 0);
                assert_eq!(statement, "RETURN 1");
                assert!(!*started, "QueryStarted has not arrived yet");
                assert!(tag.is_none(), "autocommit carries no tag");
            }
            RunState::Idle => panic!("a query is in flight"),
        }
        assert_eq!(s.run.running_id(), Some(0));
        // Draining the query falls Idle — the per-statement facts go with the variant.
        complete(&mut s, 0, 1);
        assert_eq!(s.run.running_id(), None);
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
        let effects = run_command(&mut s, ":quit");
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

    /// Shift+Tab (BackTab), normalised to Tab + shift by the IO edge.
    fn shift_tab() -> Key {
        Key { code: KeyCode::Tab, ctrl: false, alt: false, shift: true }
    }

    #[test]
    fn shift_tab_switches_focus_between_editor_and_results() {
        // ADR 0017: Shift+Tab is the focus-switch (with the popup closed); plain Tab
        // never switches focus.
        let mut s = wb();
        assert_eq!(s.focus, Focus::Editor);
        update(&mut s, Event::Key(shift_tab()));
        assert_eq!(s.focus, Focus::Results);
        update(&mut s, Event::Key(shift_tab()));
        assert_eq!(s.focus, Focus::Editor, "and back again from the results pane");
    }

    #[test]
    fn plain_tab_on_an_empty_editor_does_not_switch_focus() {
        // Empty prefix → no candidates → Tab is a no-op (no focus change, ADR 0017).
        let mut s = wb();
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert_eq!(s.focus, Focus::Editor, "Tab no longer cycles focus");
        assert!(s.completion().is_none(), "and opened no popup");
    }

    #[test]
    fn quit_works_from_the_results_pane_too() {
        let mut s = wb();
        update(&mut s, Event::Key(shift_tab())); // focus results
        assert_eq!(
            update(&mut s, Event::Key(Key::plain(KeyCode::Esc))),
            vec![Effect::Quit]
        );
    }

    // --- modal command line (ADR 0017) ---------------------------------------

    #[test]
    fn colon_on_an_empty_editor_opens_the_command_line_with_a_colon_present() {
        let mut s = wb();
        update(&mut s, Event::Key(Key::char(':')));
        let cl = s.command_line().expect("the command line opened");
        assert_eq!(cl.content, ":", "seeded with the colon prompt");
        assert_eq!(cl.prior_focus, Focus::Editor);
        assert_eq!(s.editor.buffer(), "", "the colon never reached the editor");
    }

    #[test]
    fn colon_on_a_whitespace_only_editor_opens_the_command_line() {
        let mut s = wb();
        type_str(&mut s, "  ");
        update(&mut s, Event::Key(Key::char(':')));
        assert!(s.command_line().is_some(), "whitespace counts as empty");
    }

    #[test]
    fn colon_after_cypher_is_literal_text_in_the_editor() {
        let mut s = wb();
        type_str(&mut s, "MATCH (n");
        update(&mut s, Event::Key(Key::char(':')));
        assert!(s.command_line().is_none(), "no command line — the colon is Cypher");
        assert_eq!(s.editor.buffer(), "MATCH (n:", "the colon is inserted literally");
    }

    #[test]
    fn ctrl_g_opens_the_command_line_without_touching_the_editor() {
        let mut s = wb();
        type_str(&mut s, "MATCH (n) RETURN n");
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('g'))));
        let cl = s.command_line().expect("the command line opened");
        assert_eq!(cl.content, ":", "seeded with the colon prompt");
        assert_eq!(s.editor.buffer(), "MATCH (n) RETURN n", "the half-written query is preserved");
    }

    #[test]
    fn ctrl_g_opens_from_the_results_pane_and_esc_restores_that_focus() {
        let mut s = wb();
        s.focus = Focus::Results;
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('g'))));
        assert_eq!(s.command_line().unwrap().prior_focus, Focus::Results);
        // Esc cancels and restores the prior focus.
        update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(s.command_line().is_none(), "Esc closed the command line");
        assert_eq!(s.focus, Focus::Results, "prior focus restored");
        // Esc closed the command line, it did not quit the workbench.
    }

    #[test]
    fn enter_in_the_command_line_runs_the_command_and_restores_focus() {
        let mut s = wb();
        s.focus = Focus::Results;
        let effects = run_command(&mut s, ":begin");
        assert_eq!(effects, vec![Effect::Transaction(TxOp::Begin)]);
        assert!(s.command_line().is_none(), "the command line closed after running");
        assert_eq!(s.focus, Focus::Results, "focus returns to where it was");
    }

    #[test]
    fn a_bare_colon_in_the_command_line_cancels_silently() {
        let mut s = wb();
        update(&mut s, Event::Key(Key::char(':'))); // opens with ":"
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty(), "a bare colon runs nothing");
        assert!(s.command_line().is_none(), "and closes");
    }

    #[test]
    fn a_colon_line_submitted_in_the_editor_is_cypher_not_a_meta_command() {
        // The editor is Cypher-only (ADR 0017): a `:`-line pasted/typed and submitted
        // runs as a query, not the meta parser. `:begin` here is Cypher.
        let mut s = wb();
        // Place a `:`-line in the editor directly (a paste), then submit it.
        s.editor.set_text(":begin");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        match effects.first() {
            Some(Effect::RunQuery { query, .. }) => assert_eq!(query, ":begin"),
            other => panic!("expected the :line to run as Cypher, got {other:?}"),
        }
    }

    #[test]
    fn the_command_line_is_not_part_of_the_focus_cycle() {
        // Tab while the command line is open edits/ignores within it, never switching
        // panes — the prompt is modal.
        let mut s = wb();
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('g'))));
        let focus_before = s.focus;
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert!(s.command_line().is_some(), "still open");
        assert_eq!(s.focus, focus_before, "Tab did not switch panes");
    }

    // --- command-line completion + recall (issue 02) --------------------------

    /// Open the command line and type `rest` after the seeded `:` (no Enter).
    fn open_and_type(state: &mut WorkbenchState, rest: &str) {
        update(state, Event::Key(Key::ctrl(KeyCode::Char('g'))));
        type_str(state, rest);
    }

    #[test]
    fn tab_completes_a_command_name_in_the_command_line() {
        let mut s = wb();
        open_and_type(&mut s, "beg");
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert_eq!(s.command_line().unwrap().content, ":begin");
    }

    #[test]
    fn tab_with_no_command_candidates_is_a_no_op_and_keeps_focus() {
        let mut s = wb();
        open_and_type(&mut s, "zzzq");
        let focus_before = s.focus;
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        // The content is unchanged and the command line is still open (no focus swap).
        assert_eq!(s.command_line().unwrap().content, ":zzzq");
        assert_eq!(s.focus, focus_before);
    }

    #[test]
    fn repeated_tab_cycles_through_matching_commands() {
        let mut s = wb();
        // `:s` matches several commands (set, source, save, saved, sysinfo, …).
        open_and_type(&mut s, "s");
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        let first = s.command_line().unwrap().content.clone();
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        let second = s.command_line().unwrap().content.clone();
        assert_ne!(first, second, "a repeated Tab advances to the next candidate");
        assert!(first.starts_with(":s") && second.starts_with(":s"));
    }

    #[test]
    fn an_edit_after_completion_resets_the_menu() {
        let mut s = wb();
        open_and_type(&mut s, "se");
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab))); // → :set (or first :se… match)
        assert!(s.command_line().unwrap().completion.is_some());
        update(&mut s, Event::Key(Key::char('x')));
        assert!(s.command_line().unwrap().completion.is_none(), "an edit drops the menu");
    }

    #[test]
    fn up_recalls_a_past_command_and_down_restores_the_edited_line() {
        let mut s = wb();
        run_command(&mut s, ":begin");
        // Open a fresh command line, type something, then recall.
        open_and_type(&mut s, "roll");
        update(&mut s, Event::Key(Key::plain(KeyCode::Up)));
        assert_eq!(s.command_line().unwrap().content, ":begin", "older command recalled");
        // Down past the newest restores the in-progress line.
        update(&mut s, Event::Key(Key::plain(KeyCode::Down)));
        assert_eq!(s.command_line().unwrap().content, ":roll", "the edited line is restored");
    }

    #[test]
    fn command_recall_walks_multiple_entries() {
        let mut s = wb();
        run_command(&mut s, ":begin");
        run_command(&mut s, ":commit");
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('g'))));
        update(&mut s, Event::Key(Key::plain(KeyCode::Up))); // newest → :commit
        assert_eq!(s.command_line().unwrap().content, ":commit");
        update(&mut s, Event::Key(Key::plain(KeyCode::Up))); // older → :begin
        assert_eq!(s.command_line().unwrap().content, ":begin");
    }

    #[test]
    fn the_command_history_is_distinct_from_the_query_history() {
        let mut s = wb();
        submit_query(&mut s, "RETURN 1;"); // a Cypher submission → query history
        run_command(&mut s, ":begin"); // a command → command history
        assert_eq!(s.command_history, vec![":begin".to_string()]);
        assert!(
            s.history_entries.iter().all(|e| !e.starts_with(':')),
            "the query history holds no :-commands: {:?}",
            s.history_entries
        );
        assert!(s.history_entries.contains(&"RETURN 1;".to_string()));
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
        match s.detail() {
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
        assert_eq!(s.detail(), Some(&Value::String("Ada".into())), "overlay holds the value");
    }

    #[test]
    fn esc_dismisses_the_overlay_and_keeps_the_table_selection() {
        let mut s = with_one_cell(Value::Integer(7));
        update(&mut s, Event::Key(Key::plain(KeyCode::Enter))); // open
        s.shown_mut().unwrap().selected_row = 0;
        update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(s.detail().is_none(), "overlay closed");
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
        assert_eq!(s.detail_scroll(), Some(2));
        update(&mut s, Event::Key(Key::plain(KeyCode::Up)));
        assert_eq!(s.detail_scroll(), Some(1));
    }

    // --- export (slice 09) --------------------------------------------------

    #[test]
    fn e_opens_the_export_prompt_and_enter_emits_the_export_effect() {
        use crate::OutputFormat;
        let mut s = with_one_cell(Value::String("Ada".into()));
        update(&mut s, Event::Key(Key::char('e')));
        assert!(s.export().is_some(), "prompt opened");
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
        assert!(s.export().is_none(), "prompt closed on confirm");
    }

    #[test]
    fn export_with_a_blank_path_keeps_the_prompt_open() {
        let mut s = with_one_cell(Value::Integer(1));
        update(&mut s, Event::Key(Key::char('e')));
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty(), "no export with a blank path");
        assert!(s.export().is_some(), "prompt stays open");
    }

    #[test]
    fn esc_cancels_the_export_prompt_without_quitting() {
        let mut s = with_one_cell(Value::Integer(1));
        update(&mut s, Event::Key(Key::char('e')));
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(effects.is_empty(), "Esc cancels, does not quit");
        assert!(s.export().is_none());
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
        let completion = s.completion().expect("popup open");
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
        assert!(s.completion().is_none(), "popup closed after insert");
    }

    #[test]
    fn down_cycles_the_selection_and_wraps() {
        let mut s = wb();
        type_str(&mut s, "RE"); // several keyword matches
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        let count = s.completion().unwrap().candidates.len();
        assert!(count > 1, "needs multiple candidates to cycle");
        update(&mut s, Event::Key(Key::plain(KeyCode::Down)));
        assert_eq!(s.completion().unwrap().selected, 1);
        update(&mut s, Event::Key(Key::plain(KeyCode::Up)));
        update(&mut s, Event::Key(Key::plain(KeyCode::Up)));
        assert_eq!(s.completion().unwrap().selected, count - 1, "wraps past the top");
    }

    #[test]
    fn esc_dismisses_the_completion_popup() {
        let mut s = wb();
        type_str(&mut s, "MAT");
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(s.completion().is_none());
    }

    #[test]
    fn tab_with_no_completable_prefix_is_a_no_op() {
        // An empty prefix offers nothing (the static completer declines it). Tab is
        // completion-only now (ADR 0017), so it opens no popup and never moves focus.
        let mut s = wb();
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert!(s.completion().is_none());
        assert_eq!(s.focus, Focus::Editor);
    }

    #[test]
    fn shift_tab_cycles_to_the_previous_candidate_when_the_popup_is_open() {
        let mut s = wb();
        type_str(&mut s, "RE"); // several keyword matches
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab))); // open at 0
        let count = s.completion().unwrap().candidates.len();
        assert!(count > 1, "needs multiple candidates");
        // Shift+Tab steps backwards, wrapping to the last candidate.
        update(&mut s, Event::Key(shift_tab()));
        assert_eq!(s.completion().unwrap().selected, count - 1, "wrapped to the last");
        // And forward again with Tab.
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert_eq!(s.completion().unwrap().selected, 0);
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
        let effects = run_command(&mut s, ":param y $x + 1");
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
        run_command(&mut s, ":params");
        assert_eq!(s.drawer, Some(DrawerKind::Params));
        // `:params clear` empties the store.
        run_command(&mut s, ":params clear");
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
        let effects = run_command(&mut s, ":set display vertical");
        assert!(effects.is_empty(), "a setting change runs no query");
        assert_eq!(s.settings.display, DisplayMode::Vertical);
        assert_eq!(s.editor.buffer(), "", "the command is consumed");
        assert!(s.status.message.contains("display = vertical"));
    }

    #[test]
    fn bare_set_lists_the_settings_in_the_status() {
        let mut s = wb();
        run_command(&mut s, ":set");
        assert!(s.status.message.contains("display = auto"), "status: {}", s.status.message);
    }

    #[test]
    fn an_invalid_setting_is_reported_without_losing_the_session() {
        use mgconsole_core::DisplayMode;
        let mut s = wb();
        run_command(&mut s, ":set display grid");
        assert!(s.status.message.contains("error"), "status: {}", s.status.message);
        assert_eq!(s.settings.display, DisplayMode::Auto, "unchanged on error");
        // The session survives: a following query still runs.
        let id = submit_query(&mut s, "RETURN 1;");
        assert_eq!(id, 0);
    }

    #[test]
    fn set_does_not_touch_the_param_store() {
        let mut s = wb();
        run_command(&mut s, ":set display vertical");
        assert!(s.params.is_empty(), ":set must not populate the :param store");
    }

    #[test]
    fn set_readonly_on_marks_state_and_emits_the_effect() {
        let mut s = wb();
        let effects = run_command(&mut s, ":set readonly on");
        assert!(s.read_only, "state marked read-only");
        assert_eq!(effects, vec![Effect::SetReadOnly(true)], "session told to apply it");
        assert_eq!(s.editor.buffer(), "", "command consumed");
    }

    #[test]
    fn set_readonly_off_at_runtime_is_refused() {
        let mut s = wb();
        s.read_only = true;
        let effects = run_command(&mut s, ":set readonly off");
        assert!(s.read_only, "still read-only — runtime off is refused");
        assert!(effects.is_empty(), "no effect on a refused change");
        assert!(s.status.message.contains("connect time"), "status: {}", s.status.message);
    }

    #[test]
    fn bare_set_lists_readonly_in_the_status() {
        let mut s = wb();
        run_command(&mut s, ":set");
        assert!(s.status.message.contains("readonly = off"), "status: {}", s.status.message);
    }

    // --- explicit transactions (issue 05) -----------------------------------

    #[test]
    fn begin_emits_a_transaction_effect_and_clears_the_editor() {
        let mut s = wb();
        let effects = run_command(&mut s, ":begin");
        assert_eq!(effects, vec![Effect::Transaction(TxOp::Begin)]);
        assert_eq!(s.editor.buffer(), "", "command consumed");
    }

    #[test]
    fn commit_and_rollback_emit_their_effects() {
        let mut s = wb();
        assert_eq!(
            run_command(&mut s, ":commit"),
            vec![Effect::Transaction(TxOp::Commit)]
        );
        assert_eq!(
            run_command(&mut s, ":rollback"),
            vec![Effect::Transaction(TxOp::Rollback)]
        );
    }

    #[test]
    fn a_transaction_command_is_refused_while_a_query_runs() {
        let mut s = wb();
        submit_query(&mut s, "MATCH (n) RETURN n;"); // now Running
        let effects = run_command(&mut s, ":begin");
        assert!(effects.is_empty(), "no tx op while a query is in flight");
        assert!(s.status.message.contains("busy"), "status: {}", s.status.message);
    }

    // --- transaction episodes + correlated history (issue 04) -----------------

    /// Open a transaction: run `:begin` and deliver the Session's `Open` result, as
    /// the edge would, so the reducer opens a new episode.
    fn begin_tx(state: &mut WorkbenchState) {
        run_command(state, ":begin");
        update(
            state,
            Event::TransactionApplied {
                state: mgconsole_core::TransactionState::Open,
                message: Some("transaction open".to_string()),
            },
        );
    }

    /// End a transaction with `op` (`:commit`/`:rollback`) and deliver the resulting
    /// `Auto` state, so the reducer resolves the episode.
    fn end_tx(state: &mut WorkbenchState, op: &str) {
        run_command(state, op);
        update(
            state,
            Event::TransactionApplied {
                state: mgconsole_core::TransactionState::Auto,
                message: Some("transaction ended".to_string()),
            },
        );
    }

    /// Run a query to completion with `rows` rows (tagging it via the open episode).
    /// Clears the editor first, since submit keeps the buffer for re-run.
    fn run_rows(state: &mut WorkbenchState, query: &str, rows: usize) {
        state.editor.clear();
        let id = submit_query(state, query);
        complete(state, id, rows);
    }

    #[test]
    fn statements_in_an_episode_are_numbered_and_autocommit_is_untagged() {
        let mut s = wb();
        // An autocommit query carries no tag.
        run_rows(&mut s, "RETURN 0;", 1);
        assert!(s.history[0].tx_tag.is_none(), "autocommit is untagged");

        begin_tx(&mut s);
        assert_eq!(s.tx_episode, 1, "the first :begin opens episode 1");
        run_rows(&mut s, "RETURN 1;", 1);
        run_rows(&mut s, "RETURN 2;", 1);
        let tag1 = s.history[1].tx_tag.expect("tagged");
        let tag2 = s.history[2].tx_tag.expect("tagged");
        assert_eq!((tag1.episode, tag1.ordinal), (1, 1));
        assert_eq!((tag2.episode, tag2.ordinal), (1, 2), "ordinals increment per statement");
        assert_eq!(tag1.disposition, Disposition::Open);
    }

    #[test]
    fn ordinals_continue_across_buffers_within_one_episode() {
        let mut s = wb();
        begin_tx(&mut s);
        run_rows(&mut s, "RETURN 1;", 1); // buffer 1, stmt 1
        new_buffer(&mut s);
        run_rows(&mut s, "RETURN 2;", 1); // buffer 2, stmt 2 (same episode)
        let here = s.history[0].tx_tag.expect("buffer 2 tagged");
        assert_eq!((here.episode, here.ordinal), (1, 2), "the ordinal continued across Buffers");
        // Buffer 1 kept stmt 1.
        prev_buffer(&mut s);
        let there = s.history[0].tx_tag.expect("buffer 1 tagged");
        assert_eq!((there.episode, there.ordinal), (1, 1));
    }

    #[test]
    fn commit_resolves_every_episode_entry_retroactively_across_buffers() {
        let mut s = wb();
        begin_tx(&mut s);
        run_rows(&mut s, "RETURN 1;", 1); // buffer 1
        new_buffer(&mut s);
        run_rows(&mut s, "RETURN 2;", 1); // buffer 2
        end_tx(&mut s, ":commit");
        // Buffer 2's entry is resolved...
        assert_eq!(s.history[0].tx_tag.unwrap().disposition, Disposition::Committed);
        // ...and so is buffer 1's, retroactively.
        prev_buffer(&mut s);
        assert_eq!(s.history[0].tx_tag.unwrap().disposition, Disposition::Committed);
    }

    #[test]
    fn a_rolled_back_entry_keeps_its_rows_and_is_marked_undone() {
        let mut s = wb();
        begin_tx(&mut s);
        run_rows(&mut s, "MATCH (n) RETURN n;", 3);
        end_tx(&mut s, ":rollback");
        let entry = &s.history[0];
        assert_eq!(entry.tx_tag.unwrap().disposition, Disposition::RolledBack);
        assert_eq!(entry.rows.len(), 3, "the rows are kept, just marked undone");
    }

    #[test]
    fn a_poisoned_transaction_resolves_to_rolled_back() {
        let mut s = wb();
        begin_tx(&mut s);
        // A statement fails inside the transaction, poisoning it.
        let id = submit_query(&mut s, "BAD;");
        let boom = Error::Query(QueryError {
            code: "Memgraph.ClientError.MemgraphError.SyntaxError".to_string(),
            message: "bad cypher".to_string(),
        });
        update(&mut s, Event::QueryFailed { id, error: boom });
        // The edge syncs the poisoned marker (no pending op → no resolution).
        update(
            &mut s,
            Event::TransactionApplied {
                state: mgconsole_core::TransactionState::Failed,
                message: None,
            },
        );
        assert_eq!(s.tx, mgconsole_core::TransactionState::Failed);
        assert_eq!(s.history[0].tx_tag.unwrap().disposition, Disposition::Open, "still open until rolled back");
        // Only :rollback recovers a poisoned transaction — it resolves the episode.
        end_tx(&mut s, ":rollback");
        assert_eq!(s.history[0].tx_tag.unwrap().disposition, Disposition::RolledBack);
    }

    #[test]
    fn a_second_episode_increments_the_number_and_resets_ordinals() {
        let mut s = wb();
        begin_tx(&mut s);
        run_rows(&mut s, "RETURN 1;", 1);
        end_tx(&mut s, ":commit");
        begin_tx(&mut s);
        run_rows(&mut s, "RETURN 2;", 1);
        let tag = s.history[1].tx_tag.expect("tagged");
        assert_eq!((tag.episode, tag.ordinal), (2, 1), "a new episode, ordinal restarts");
    }

    // --- :connect (issue 07) ------------------------------------------------

    #[test]
    fn connect_emits_a_connect_effect_and_warns_about_an_open_transaction() {
        let mut s = wb();
        s.tx = mgconsole_core::TransactionState::Open;
        let effects = run_command(&mut s, ":connect prod");
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
        let effects = run_command(&mut s, ":source setup.cypher");
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
        let id = s.run.running_id().expect("should be running");
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
        let effects = run_command(&mut s, ":o csv /tmp/wb_o.csv");
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
        let id = s.run.running_id().expect("running");
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
        let effects = run_command(&mut s, ":watch 1s");
        assert!(matches!(effects.first(), Some(Effect::RunQuery { query, .. }) if query == "RETURN 1"));
        assert!(s.watch.is_some());
        let id = s.run.running_id().expect("running");
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
        let id = s.run.running_id().expect("running");
        complete(&mut s, id, 1);
        assert_eq!(s.history.len(), after_first, "the snapshot was replaced, not appended");
    }

    #[test]
    fn any_key_stops_watching() {
        let mut s = wb();
        let id = submit_query(&mut s, "RETURN 1;");
        complete(&mut s, id, 1);
        s.editor.clear();
        run_command(&mut s, ":watch 1s");
        let id = s.run.running_id().expect("running");
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
        let effects = run_command(&mut s, ":watch");
        assert!(effects.is_empty());
        assert!(s.watch.is_none());
        assert!(s.status.message.contains("refused while a transaction is open"));
    }

    #[test]
    fn sysinfo_runs_the_status_queries_as_a_batch() {
        let mut s = wb();
        let effects = run_command(&mut s, ":sysinfo");
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
        let effects = run_command(&mut s, ":use analytics");
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
        let effects = run_command(&mut s, ":connect prod");
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
    fn ctrl_n_toggles_the_summary_drawer() {
        let mut s = wb();
        // Ctrl+N toggles the summary drawer (it moved off Ctrl+Y, now redo; ADR 0016).
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('n'))));
        assert_eq!(s.drawer, Some(DrawerKind::Summary));
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('n'))));
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
        assert!(s.detail().is_some(), "detail overlay open");
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
        assert!(s.search().is_some(), "search opened");
        type_str(&mut s, "ap");
        let search = s.search().expect("open");
        assert_eq!(search.matches, vec![0, 2], "apple and apricot match 'ap'");
        assert_eq!(search.current, Some(0));
        // The selection moved onto the first match.
        assert_eq!(s.shown().unwrap().selected_row, 0);
        // Esc clears search and restores the full view.
        update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(s.search().is_none(), "cleared");
    }

    #[test]
    fn search_is_case_insensitive() {
        let mut s = with_text_rows(&["Apple", "BANANA"], 10);
        open_search_overlay(&mut s);
        type_str(&mut s, "ban");
        assert_eq!(s.search().unwrap().matches, vec![1]);
    }

    #[test]
    fn next_and_previous_match_navigation_moves_the_selection() {
        let mut s = with_text_rows(&["apple", "banana", "apricot", "cherry", "grape"], 10);
        open_search_overlay(&mut s);
        type_str(&mut s, "a"); // apple(0), banana(1), apricot(2), grape(4)
        assert_eq!(s.search().unwrap().matches, vec![0, 1, 2, 4]);
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
        assert!(s.search().unwrap().filter_only, "filter on");
        // Tab again restores the full view (toggles off).
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert!(!s.search().unwrap().filter_only, "filter off");
    }

    #[test]
    fn backspace_re_matches_live() {
        let mut s = with_text_rows(&["apple", "apricot", "banana"], 10);
        open_search_overlay(&mut s);
        type_str(&mut s, "app");
        assert_eq!(s.search().unwrap().matches, vec![0]);
        update(&mut s, Event::Key(Key::plain(KeyCode::Backspace)));
        // Now "ap" matches both apple and apricot.
        assert_eq!(s.search().unwrap().matches, vec![0, 1]);
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
        assert!(s.search().is_none(), "nothing to search");
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
    fn close_is_a_workbench_command_not_a_shared_meta_command() {
        // `:close` is parsed by the Workbench's own layer; it is not part of the
        // shared meta vocabulary (CONTEXT.md "Workbench command").
        assert_eq!(
            crate::repl::meta_command(":close"),
            Some(crate::repl::MetaCommand::Unknown(":close".to_string()))
        );
        assert_eq!(workbench_command(":close"), Some(WorkbenchCommand::Close));
        assert_eq!(workbench_command(":param x 1"), None, "a shared meta-command is not a workbench command");
    }

    #[test]
    fn close_cancels_the_query_when_closing_its_owning_buffer() {
        let mut s = wb();
        new_buffer(&mut s); // two buffers; active is buffer 2
        let id = submit_query(&mut s, "RETURN 1;"); // runs in (and belongs to) buffer 2
        assert_eq!(s.running_buffer, Some(1));
        let effects = submit_meta(&mut s, ":close"); // close the owning buffer
        assert!(effects.contains(&Effect::Cancel { id }), "closing the owning buffer cancels its query");
        assert_eq!(s.buffer_count(), 1);
        assert!(matches!(s.run, RunState::Idle), "the session falls idle");
        assert!(s.running_buffer.is_none(), "no buffer owns a query any more");
    }

    #[test]
    fn close_of_an_idle_buffer_leaves_another_buffers_query_running() {
        let mut s = wb();
        let id = submit_query(&mut s, "RETURN 1;"); // runs in buffer 1
        assert_eq!(s.running_buffer, Some(0));
        new_buffer(&mut s); // buffer 2 (idle) active; running index fixed up to 0
        assert_eq!(s.running_buffer, Some(0));
        let effects = submit_meta(&mut s, ":close"); // close idle buffer 2
        assert!(effects.is_empty(), "no cancel — a different buffer owns the query");
        assert_eq!(s.buffer_count(), 1);
        assert!(matches!(s.run, RunState::Running { .. }), "buffer 1's query keeps running");
        assert_eq!(s.running_buffer, Some(0), "still owned by buffer 1 (now the only buffer)");
        // Its result still streams into the surviving buffer.
        update(&mut s, Event::QueryStarted { id, header: vec!["n".to_string()] });
        update(&mut s, Event::RecordArrived { id, record: one_row() });
        assert_eq!(s.history.len(), 1);
        assert_eq!(s.history[0].rows.len(), 1);
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
        lay_out_tabs(&mut s);
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
        // The draw would lay out the tab bar; do the same so the click has hits.
        lay_out_tabs(&mut s);
        // Click the first tab (its column range starts at the bar's left edge).
        click(&mut s, 1, 0);
        assert_eq!(s.active, 0, "clicked the first tab");
        assert_eq!(s.editor.buffer(), "ONE");
    }

    // --- Auto-titled tabs + windowed bar (issue 07) ---------------------------

    #[test]
    fn a_tab_is_auto_titled_from_its_query_then_editor_then_number() {
        let mut s = wb();
        // Buffer 1 has a result, so its title comes from the originating query.
        s.history.push(CurrentResult::new("MATCH (n) RETURN n".into(), vec!["n".into()]));
        assert!(s.buffer_title(0).contains("MATCH"), "titled from the query: {}", s.buffer_title(0));
        // A fresh buffer with neither result nor text falls back to its number.
        new_buffer(&mut s);
        assert_eq!(s.buffer_title(1), "2", "empty buffer titled by number");
        // Typing gives it an editor-line title.
        type_str(&mut s, "CREATE (x)");
        assert!(s.buffer_title(1).contains("CREATE"), "titled from the editor line");
    }

    #[test]
    fn the_running_buffer_tab_is_marked_in_its_title() {
        let mut s = wb();
        new_buffer(&mut s);
        s.running_buffer = Some(0);
        assert!(s.buffer_title(0).starts_with('•'), "a running buffer is marked: {}", s.buffer_title(0));
        assert!(!s.buffer_title(1).starts_with('•'), "an idle buffer is not");
    }

    #[test]
    fn the_tab_bar_windows_to_keep_the_active_tab_visible() {
        let mut s = wb();
        for _ in 0..9 {
            new_buffer(&mut s); // 10 buffers; active is the last
        }
        let bar = s.layout_tabs(Rect::new(0, 0, 20, 1));
        assert!(bar.tabs.iter().any(|t| t.index == s.active), "the active tab is in the window");
        assert!(bar.left_more, "tabs off the left edge are marked");
        assert!(bar.tabs.first().unwrap().index > 0, "the bar scrolled past the first tab");
    }

    #[test]
    fn a_tab_click_maps_to_the_right_buffer_under_windowing() {
        let mut s = wb();
        for _ in 0..9 {
            new_buffer(&mut s);
        }
        let area = Rect::new(0, 0, 20, 1);
        s.tabbar_area = area;
        s.tab_hits = s.layout_tabs(area).tabs;
        // Click a visible tab that isn't the active one; it selects that Buffer
        // even though the bar is scrolled (the hit accounts for the offset).
        let target = s.tab_hits.iter().find(|t| t.index != s.active).cloned().expect("another tab");
        click(&mut s, target.col + 1, 0);
        assert_eq!(s.active, target.index, "the click mapped to the windowed tab's buffer");
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

    /// Lay out the tab bar over a wide bar as the draw would (issue 07), so a
    /// tab-bar click has hit boxes to map against without a terminal.
    fn lay_out_tabs(state: &mut WorkbenchState) {
        let area = Rect::new(0, 0, 80, 1);
        state.tabbar_area = area;
        state.tab_hits = state.layout_tabs(area).tabs;
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
        assert!(s.detail().is_some(), "the cell-expand overlay opened");
    }

    #[test]
    fn a_click_while_search_is_open_does_not_open_a_second_overlay() {
        // Regression (the Modal sum type): the mouse is ignored while any Modal is
        // open, so a click on a result cell while in-result search is open no longer
        // opens the cell-detail overlay on top of it — "two overlays open" is
        // unrepresentable now.
        let mut s = with_mouse_layout(&["apple", "banana", "cherry"]);
        s.focus = Focus::Results;
        open_search_overlay(&mut s);
        assert_eq!(s.modal_kind(), Some(ModalKind::Search), "search is the open modal");
        click(&mut s, 3, 8); // a cell that would otherwise open cell-detail
        assert_eq!(s.modal_kind(), Some(ModalKind::Search), "still just search — no detail stacked");
    }

    #[test]
    fn clicking_the_header_row_only_focuses_without_selecting() {
        let mut s = with_mouse_layout(&["apple", "banana"]);
        click(&mut s, 1, 6); // the header row
        assert_eq!(s.focus, Focus::Results);
        assert!(s.detail().is_none(), "no cell expanded from a header click");
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
        s.modal = Some(Modal::Detail { value: Value::Integer(1), scroll: 0 });
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

    /// Run a `:`-meta command through the modal Command line (ADR 0017). The editor
    /// is Cypher-only now, so a command is issued from the command line, not by
    /// submitting it in the editor.
    fn submit_meta(state: &mut WorkbenchState, command: &str) -> Vec<Effect> {
        run_command(state, command)
    }

    #[test]
    fn save_with_a_query_stores_a_template_and_requests_a_persist() {
        let mut s = wb();
        let effects = submit_meta(&mut s, ":save recent MATCH (n) RETURN $limit");
        // The $param placeholder is kept verbatim — a template, not a frozen value.
        assert_eq!(s.queries.get("recent"), Some("MATCH (n) RETURN $limit"));
        assert_eq!(effects, vec![Effect::PersistQuery("recent".to_string())]);
        assert!(s.status.message.contains("saved 'recent'"));
        assert_eq!(s.editor.buffer(), "", "a command is consumed");
    }

    #[test]
    fn save_with_no_query_saves_the_last_query() {
        let mut s = wb();
        submit_query(&mut s, "RETURN 1;");
        let effects = submit_meta(&mut s, ":save one");
        assert_eq!(s.queries.get("one"), Some("RETURN 1"));
        assert_eq!(effects, vec![Effect::PersistQuery("one".to_string())]);
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
        assert_eq!(effects, vec![Effect::PersistQuery("a".to_string())]);
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
        assert!(s.help_open(), "the help overlay opened");
        assert_eq!(s.editor.buffer(), "", "the command was consumed");
        // While open, Down scrolls and Esc closes (the overlay owns input).
        update(&mut s, Event::Key(Key::plain(KeyCode::Down)));
        assert_eq!(s.help_scroll(), Some(1));
        update(&mut s, Event::Key(Key::plain(KeyCode::Esc)));
        assert!(!s.help_open(), "Esc dismisses the overlay rather than quitting");
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
            "ctrl+x".to_string(),
        )]));
        let rebound = keybindings_help(&keys);
        assert!(rebound.contains("ctrl+x"), "the rebound chord is shown: {rebound}");
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
        // The light built-in is selectable and tunes both categories and chrome.
        submit_meta(&mut s, ":set theme light");
        assert_eq!(s.status.message, "theme = light");
        assert_eq!(s.palette.color(HighlightCategory::Keyword), ThemeColor::Blue);
        assert_eq!(s.palette.border, ThemeColor::Blue, "light tunes the chrome too");
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

    // --- Discoverability: ? help + live-key hint (issue 10) -------------------

    #[test]
    fn question_mark_opens_help_when_the_editor_is_empty() {
        let mut s = wb();
        update(&mut s, Event::Key(Key::char('?')));
        assert!(s.help_open(), "? opened the help overlay on an empty editor");
    }

    #[test]
    fn question_mark_opens_help_when_the_results_pane_is_focused() {
        let mut s = wb();
        type_str(&mut s, "MATCH (n) RETURN n"); // a non-empty editor…
        s.focus = Focus::Results; // …but focus is on the results pane
        update(&mut s, Event::Key(Key::char('?')));
        assert!(s.help_open(), "? opened help from the results pane");
        assert_eq!(s.editor.buffer(), "MATCH (n) RETURN n", "the query was not edited");
    }

    #[test]
    fn question_mark_inserts_into_a_non_empty_query() {
        let mut s = wb();
        type_str(&mut s, "MATCH");
        update(&mut s, Event::Key(Key::char('?')));
        assert!(!s.help_open(), "? did not open help mid-query");
        assert_eq!(s.editor.buffer(), "MATCH?", "? was inserted into the query");
    }

    #[test]
    fn the_status_hint_points_at_help_and_reflects_a_rebinding() {
        use crate::theme::resolve_keys;
        let overrides = std::collections::BTreeMap::from([
            ("format-buffer".to_string(), "ctrl+x".to_string()),
        ]);
        let (keys, _warnings) = resolve_keys(&overrides);
        let hint = status_hint(&keys, "Alt+Enter");
        assert!(hint.contains("? help"), "the hint points at help: {hint}");
        assert!(hint.contains("ctrl+x"), "the hint reflects the rebound format chord: {hint}");
    }

    #[test]
    fn the_help_overlay_reflects_a_rebinding() {
        use crate::theme::resolve_keys;
        let overrides = std::collections::BTreeMap::from([
            ("next-buffer".to_string(), "ctrl+n".to_string()),
        ]);
        let (keys, _warnings) = resolve_keys(&overrides);
        let help = keybindings_help(&keys);
        assert!(help.contains("ctrl+n"), "help shows the rebound next-buffer chord");
        // The remapped defaults and the :close command are present too.
        assert!(help.contains(":close"), "help lists the :close command");
    }

    // --- Undoable buffer replacement (issue 09) -------------------------------

    fn undo(state: &mut WorkbenchState) {
        update(state, Event::Key(Key::ctrl(KeyCode::Char('z'))));
    }
    fn redo(state: &mut WorkbenchState) {
        update(state, Event::Key(Key::ctrl(KeyCode::Char('y'))));
    }

    #[test]
    fn auto_format_is_undoable_and_redoable() {
        let mut s = wb();
        type_str(&mut s, "match (n) return n");
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('l')))); // auto-format
        assert_eq!(s.editor.buffer(), "MATCH (n)\nRETURN n", "formatted");
        undo(&mut s);
        assert_eq!(s.editor.buffer(), "match (n) return n", "undo restores the pre-format text");
        redo(&mut s);
        assert_eq!(s.editor.buffer(), "MATCH (n)\nRETURN n", "redo re-applies the format");
    }

    #[test]
    fn history_recall_over_unsaved_text_is_undoable() {
        let mut s = wb();
        type_str(&mut s, "draft I am editing");
        // Recall replaces the buffer (slice 17); it must not lose the draft.
        s.editor.set_text("MATCH (n) RETURN n");
        assert_eq!(s.editor.buffer(), "MATCH (n) RETURN n");
        undo(&mut s);
        assert_eq!(s.editor.buffer(), "draft I am editing", "undo restores the unsaved draft");
    }

    #[test]
    fn ordinary_typed_edits_remain_undoable() {
        let mut s = wb();
        type_str(&mut s, "abc");
        undo(&mut s);
        assert_ne!(s.editor.buffer(), "abc", "an undo steps back a typed edit");
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
        // Rebind toggle-params to Ctrl-X; the default Ctrl-P no longer toggles it.
        let mut config = WorkbenchConfig::default();
        let (keys, warnings) = crate::theme::resolve_keys(&BTreeMap::from([(
            "toggle-params".to_string(),
            "ctrl+x".to_string(),
        )]));
        assert!(warnings.is_empty());
        config.keys = keys;
        let mut s = WorkbenchState::new(config, true);

        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('x'))));
        assert_eq!(s.drawer, Some(DrawerKind::Params), "the rebound chord works");

        // The old default chord is now unbound: Ctrl-P is ordinary input, not a toggle.
        s.drawer = None;
        s.focus = Focus::Editor;
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('p'))));
        assert_eq!(s.drawer, None, "the freed default chord no longer toggles");
    }
}
