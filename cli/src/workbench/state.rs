//! The pure workbench state.
//!
//! Built up across slices: slice 01 ships the shell subset (editor, focus,
//! status, colour, config); slices 02–18 add fields (run state, result history,
//! completion, schema, parameters, drawers) without reshaping the type. The
//! state holds no terminal and no Session — it is driven by [`super::update`]
//! and rendered by [`super::draw`], the analogue of the REPL's loop/IO split.

use std::collections::{BTreeMap, VecDeque};

use mgconsole_core::{Record, Summary, Value};
use tui_textarea::{Input, Key as TaKey, TextArea};

use super::effect::ExportFormat;
use super::event::{Key, KeyCode};

/// The export prompt's state (slice 09): the chosen format and the destination
/// path being typed.
#[derive(Debug, Clone)]
pub struct ExportPrompt {
    pub format: ExportFormat,
    pub path: String,
}

impl Default for ExportPrompt {
    fn default() -> Self {
        Self {
            format: ExportFormat::Csv,
            path: String::new(),
        }
    }
}

/// The complete workbench state for the current slice.
pub struct WorkbenchState {
    /// The multiline query editor.
    pub editor: EditorState,
    /// Which pane has keyboard focus.
    pub focus: Focus,
    /// Whether a query is in flight (the one-live-result guard, ADR 0005).
    pub run: RunState,
    /// Statements from one multi-statement submit still to run, in order.
    pub pending: VecDeque<String>,
    /// The result currently on screen (rows stream into it); `None` before the
    /// first query. Slice 10 turns this into a history stack.
    pub result: Option<CurrentResult>,
    /// A spinner frame counter, advanced by ticks while a query is in flight, so
    /// the draw can show a running indicator (slice 07).
    pub spinner: usize,
    /// When `Some`, the cell-detail overlay is open showing this Value in full
    /// (slice 08); `None` is the table view.
    pub detail: Option<Value>,
    /// Vertical scroll of the detail overlay, for a Value taller than the box.
    pub detail_scroll: u16,
    /// When `Some`, the export prompt is open (slice 09): pick a format and type
    /// a destination path for the on-screen result.
    pub export: Option<ExportPrompt>,
    /// The results-table viewport height (data rows) from the last draw, cached so
    /// the reducer can page and keep the selection visible without re-deriving the
    /// layout. The draw is the only writer.
    pub viewport_rows: usize,
    /// The `:param` store bound to every query (populated in slice 16).
    pub params: BTreeMap<String, Value>,
    /// Monotonic id stamped on each query, so its lifecycle events match.
    pub next_id: u64,
    /// The transient status message (errors, hints; running/elapsed in slice 07).
    pub status: StatusLine,
    /// Whether colour is on (resolved `--color`/`NO_COLOR`); slice 05 uses it for
    /// editor highlighting. A monochrome workbench is styled, not disabled.
    pub color: bool,
    /// Frontend-local configuration.
    pub config: WorkbenchConfig,
}

impl WorkbenchState {
    /// A fresh workbench: an empty editor with focus, ready for input.
    pub fn new(config: WorkbenchConfig, color: bool) -> Self {
        Self {
            editor: EditorState::new(),
            focus: Focus::Editor,
            run: RunState::Idle,
            pending: VecDeque::new(),
            result: None,
            spinner: 0,
            detail: None,
            detail_scroll: 0,
            export: None,
            viewport_rows: 0,
            params: BTreeMap::new(),
            next_id: 0,
            status: StatusLine::default(),
            color,
            config,
        }
    }
}

/// Whether a query is in flight. While `Running`, a second submit is refused
/// (one-live-result, ADR 0005). The `id` matches the running query's lifecycle
/// events; events for any other id are stragglers and ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Running { id: u64 },
}

/// The result on screen: a column header, the rows streamed so far, and the
/// table-navigation cursor. Rendered as a navigable ratatui table (only the
/// visible window is drawn, so a huge result stays navigable).
#[derive(Debug, Default, Clone)]
pub struct CurrentResult {
    pub header: Vec<String>,
    pub rows: Vec<Record>,
    /// The selected row, for navigation and cell-expand (slice 08).
    pub selected_row: usize,
    /// The selected column.
    pub selected_col: usize,
    /// The first visible data row (scroll offset), kept so the selection stays in
    /// view; the draw renders only `rows[scroll .. scroll + viewport]`.
    pub scroll: usize,
    /// Set when the row-cap backstop was hit and further rows were dropped (a
    /// memory guard, not a usability limit — the cap is high and configurable).
    pub truncated: bool,
    /// Set when the query was cancelled mid-stream (slice 07): the rows present
    /// are a partial answer, kept on screen and labelled as such.
    pub partial: bool,
    /// The trailing summary, available once the query completes (slice 18).
    pub summary: Option<Summary>,
}

impl CurrentResult {
    /// A fresh result for a query's `header`, cursor at the top-left.
    pub fn new(header: Vec<String>) -> Self {
        Self {
            header,
            ..Self::default()
        }
    }
}

/// Which pane the keyboard drives. The results pane fills in from slice 03;
/// drawers/overlays (schema, params, summary, cell-detail) are added by their
/// slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Results,
}

/// The one-line status message. Distinct from the keybind hint, which the draw
/// composes from [`WorkbenchConfig`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub message: String,
}

/// A high backstop on rows held in memory for one result. Unlike the REPL's
/// `DEFAULT_ROW_CAP` (a usability limit on a buffered table), this is purely a
/// memory guard: only the visible window is ever drawn, so a result of this size
/// is still navigable. Configurable; reached only by a pathological result.
pub const DEFAULT_ROW_CAP: usize = 1_000_000;

/// Frontend-local configuration resolved at startup.
#[derive(Debug, Clone)]
pub struct WorkbenchConfig {
    /// The editor pane's share of the vertical split, in percent.
    pub editor_percent: u16,
    /// The universal newline key, shown in the status hint (slice 01). Ctrl/Shift
    /// +Enter on capable terminals is negotiated in slice 06.
    pub newline_hint: &'static str,
    /// The memory backstop on rows held for one result (see [`DEFAULT_ROW_CAP`]).
    pub row_cap: usize,
}

impl Default for WorkbenchConfig {
    fn default() -> Self {
        Self {
            editor_percent: 40,
            newline_hint: "Alt+Enter",
            row_cap: DEFAULT_ROW_CAP,
        }
    }
}

/// The multiline editor, wrapping a [`tui_textarea::TextArea`] for free cursor
/// movement, selection, and undo. The widget runs **headlessly** — its
/// [`input`](TextArea::input) mutates state with no terminal — so reducer tests
/// over the editor need no terminal. The reducer feeds ordinary editing keys
/// here via [`edit`](Self::edit) and reads the buffer on submit; gesture keys
/// (Enter, the newline key, quit) it handles itself.
pub struct EditorState {
    textarea: TextArea<'static>,
}

impl EditorState {
    /// An empty editor.
    pub fn new() -> Self {
        Self {
            textarea: TextArea::default(),
        }
    }

    /// Apply an ordinary editing key (character, backspace, arrows, …) to the
    /// buffer. Keys the editor does not act on are ignored. Enter and the newline
    /// key are *not* routed here — the reducer owns those gestures.
    pub fn edit(&mut self, key: Key) {
        if let Some(input) = to_input(key) {
            self.textarea.input(input);
        }
    }

    /// Insert a newline at the cursor (the universal newline gesture).
    pub fn insert_newline(&mut self) {
        self.textarea.insert_newline();
    }

    /// The whole buffer as one string, physical lines joined by `\n`.
    pub fn buffer(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Discard the buffer back to empty (Ctrl-C abandons typing when idle).
    pub fn clear(&mut self) {
        self.textarea = TextArea::default();
    }

    /// The physical lines, for the draw edge to render with per-token
    /// highlighting (tui-textarea has no per-token styling, so the workbench
    /// renders the lines itself and uses the widget only as the edit model).
    pub fn lines(&self) -> &[String] {
        self.textarea.lines()
    }

    /// The cursor position as `(row, column)` in characters, for the draw to
    /// place the terminal cursor.
    pub fn cursor(&self) -> (usize, usize) {
        self.textarea.cursor()
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

/// Translate a neutral [`Key`] into a `tui_textarea` input, or `None` for a key
/// the editor ignores. Enter/Esc are deliberately absent — the reducer handles
/// them as gestures, never as editing.
fn to_input(key: Key) -> Option<Input> {
    let ta_key = match key.code {
        KeyCode::Char(c) => TaKey::Char(c),
        KeyCode::Backspace => TaKey::Backspace,
        KeyCode::Delete => TaKey::Delete,
        KeyCode::Tab => TaKey::Tab,
        KeyCode::Left => TaKey::Left,
        KeyCode::Right => TaKey::Right,
        KeyCode::Up => TaKey::Up,
        KeyCode::Down => TaKey::Down,
        KeyCode::Home => TaKey::Home,
        KeyCode::End => TaKey::End,
        KeyCode::PageUp => TaKey::PageUp,
        KeyCode::PageDown => TaKey::PageDown,
        KeyCode::Enter | KeyCode::Esc | KeyCode::Other => return None,
    };
    Some(Input {
        key: ta_key,
        ctrl: key.ctrl,
        alt: key.alt,
        shift: key.shift,
    })
}
