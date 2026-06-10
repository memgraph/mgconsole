//! The pure workbench state.
//!
//! Built up across slices: slice 01 ships the shell subset (editor, focus,
//! status, colour, config); slices 02–18 add fields (run state, result history,
//! completion, schema, parameters, drawers) without reshaping the type. The
//! state holds no terminal and no Session — it is driven by [`super::update`]
//! and rendered by [`super::draw`], the analogue of the REPL's loop/IO split.

use std::collections::{BTreeMap, VecDeque};

use mgconsole_core::{ConnectOptions, Record, Summary, TransactionState, Value};

use crate::config::Config;
use crate::queries::NamedQueries;
use crate::theme::{builtin_palette, KeyBindings, Palette};
use ratatui::layout::Rect;
use tui_textarea::{Input, Key as TaKey, TextArea};

use crate::settings::Settings;
use crate::syntax::{word_start, Completer};

use crate::OutputFormat;
use super::event::{Key, KeyCode};
use super::plan::Plan;
use super::schema::Schema;

/// The open completion popup (slice 11): the candidates for the word under the
/// cursor, the highlighted one, and the length (in characters) of the prefix a
/// chosen candidate replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub candidates: Vec<String>,
    pub selected: usize,
    pub prefix_len: usize,
}

/// In-result search/filter state (issue 16): an open search input over the
/// currently-shown result, matching rows by substring across all cells. Operates
/// only over rows already loaded into the result view — it never re-runs the query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchState {
    /// The substring being searched for (case-insensitive).
    pub query: String,
    /// Whether the view is filtered to only matching rows (Tab toggles it).
    pub filter_only: bool,
    /// Indices into the shown result's `rows` that match the query, in order.
    pub matches: Vec<usize>,
    /// Which entry of `matches` is the active match (the selection sits on it), or
    /// `None` when there are no matches.
    pub current: Option<usize>,
}

/// The export prompt's state (slice 09): the chosen format and the destination
/// path being typed.
#[derive(Debug, Clone)]
pub struct ExportPrompt {
    pub format: OutputFormat,
    pub path: String,
}

impl Default for ExportPrompt {
    fn default() -> Self {
        Self {
            format: OutputFormat::Csv,
            path: String::new(),
        }
    }
}

/// One Buffer's own state (issue 18): its editor text, its result + result-history
/// stack, and its command-history recall position. Everything else — the Session,
/// `:param` store, Schema, Settings, the one live query — is session-global and
/// lives directly on [`WorkbenchState`], shared across all Buffers.
///
/// The *active* Buffer's state is held in the matching top-level fields of
/// `WorkbenchState` (so the reducer reads it directly); the inactive Buffers are
/// parked here in [`WorkbenchState::buffers`] and swapped in on a switch.
#[derive(Default)]
pub struct Buffer {
    pub editor: EditorState,
    pub history: Vec<CurrentResult>,
    pub view: usize,
    pub recall_index: Option<usize>,
    pub recall_saved: Option<String>,
}

/// The complete workbench state for the current slice.
// A flat aggregate of mostly-independent UI flags (focus markers, open overlays,
// colour); grouping the bools to satisfy the lint would obscure that 1:1 mapping.
#[allow(clippy::struct_excessive_bools)]
pub struct WorkbenchState {
    /// The multiline query editor.
    pub editor: EditorState,
    /// Which pane has keyboard focus.
    pub focus: Focus,
    /// Whether a query is in flight (the one-live-result guard, ADR 0005).
    pub run: RunState,
    /// Statements from one multi-statement submit still to run, in order.
    pub pending: VecDeque<String>,
    /// The navigable history of results, one entry per statement run, in order
    /// (slice 10). The in-flight query streams into the last entry; [`view`]
    /// selects which entry is shown.
    ///
    /// [`view`]: Self::view
    pub history: Vec<CurrentResult>,
    /// Index into [`history`](Self::history) of the result currently shown.
    pub view: usize,
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
    /// When `Some`, in-result search is open over the shown result (issue 16).
    pub search: Option<SearchState>,
    /// Whether the `:help` keybinding overlay is open, and how far it is scrolled.
    pub help: bool,
    pub help_scroll: u16,
    /// When `Some`, the completion popup is open (slice 11).
    pub completion: Option<Completion>,
    /// The completion candidate source(s): the static keyword/function vocabulary
    /// (slice 11), joined by a live schema source when the Schema is loaded
    /// (slice 12).
    pub completer: Completer,
    /// The fetched database Schema (slice 12): `None` when the metadata feature
    /// is off/unfetched. Backs both schema completion and the sidebar (slice 13).
    pub schema: Option<Schema>,
    /// The open side drawer, if any (slice 13+). At most one is open at a time.
    pub drawer: Option<DrawerKind>,
    /// The results-table viewport height (data rows) from the last draw, cached so
    /// the reducer can page and keep the selection visible without re-deriving the
    /// layout. The draw is the only writer.
    pub viewport_rows: usize,
    /// The inactive Buffers (issue 18), parked while another is active; the active
    /// Buffer's state lives in the top-level [`editor`](Self::editor)/
    /// [`history`](Self::history)/[`view`](Self::view) fields. `buffers[active]` is
    /// a placeholder swapped with those fields on a switch. There is always ≥1.
    pub buffers: Vec<Buffer>,
    /// The active Buffer index into [`buffers`](Self::buffers).
    pub active: usize,
    /// The tab bar's rectangle from the last draw (issue 17/18), cached for mouse
    /// hit-testing; empty when only one Buffer is open (no tab bar drawn).
    pub tabbar_area: Rect,
    /// The editor pane's inner rectangle from the last draw (issue 17), cached so
    /// the reducer can hit-test a mouse click without knowing the layout. The draw
    /// is the only writer; the reducer only reads.
    pub editor_area: Rect,
    /// The results pane's inner rectangle from the last draw (issue 17), header row
    /// included; cached for mouse hit-testing as above.
    pub results_area: Rect,
    /// The `:param` store bound to every query (populated in slice 16).
    pub params: BTreeMap<String, Value>,
    /// Monotonic id stamped on each query, so its lifecycle events match.
    pub next_id: u64,
    /// The statement of the in-flight query, carried from submit to the result
    /// entry created when the query starts (for plan detection, slice 14).
    pub running_statement: Option<String>,
    /// The Buffer that owns the in-flight query (issue 03), as an index into
    /// [`buffers`](Self::buffers). A live query streams its Records, completion,
    /// and failure into this Buffer even while another is active, so switching
    /// Buffers mid-query is always allowed — only a second submit is refused. Set
    /// when a submission's first statement begins; cleared when the Session falls
    /// idle. `None` when no query is in flight.
    pub running_buffer: Option<usize>,
    /// Persisted command history, oldest→newest, for recall (slice 17). Loaded on
    /// start and appended on each submit.
    pub history_entries: Vec<String>,
    /// The recall position into [`history_entries`](Self::history_entries), or
    /// `None` when editing the live buffer.
    pub recall_index: Option<usize>,
    /// The live buffer saved when recall began, restored on stepping past newest.
    pub recall_saved: Option<String>,
    /// The transient status message (errors, hints; running/elapsed in slice 07).
    pub status: StatusLine,
    /// Whether colour is on (resolved `--color`/`NO_COLOR`); slice 05 uses it for
    /// editor highlighting. A monochrome workbench is styled, not disabled.
    pub color: bool,
    /// The live console Settings (issue 01): seeded from config, mutated by
    /// `:set`. Shared spine with the REPL — kept distinct from [`params`].
    ///
    /// [`params`]: Self::params
    pub settings: Settings,
    /// Whether the Session is read-only (issue 04): seeded from config, turned on
    /// by `:set readonly on`. Drives the `[read-only]` status-bar marker; the
    /// off-at-runtime refusal is enforced by the reducer.
    pub read_only: bool,
    /// The explicit-transaction state (issue 05), mirrored from the Session by the
    /// `TransactionApplied` event. Drives the `[tx]`/`[tx failed]` status marker.
    pub tx: TransactionState,
    /// The active profile name (issue 03/07): seeded from config, updated by
    /// `:connect`. Shown in the status bar.
    pub profile: Option<String>,
    /// The Endpoint shown in the status bar (issue 07): seeded from config,
    /// updated by `:connect`.
    pub endpoint: String,
    /// The active Database once switched with `:use` (issue 08); `None` keeps the
    /// server default. Shown in the status bar; reset by a `:connect` swap.
    pub database: Option<String>,
    /// Set while running a `:source` batch (issue 10): a failed statement then
    /// stops the batch (drops the queue) rather than continuing, mirroring the
    /// REPL's stop-on-first-error.
    pub source_halt: bool,
    /// Active `:watch` (issue 11): re-runs its query every `period_ticks`, each
    /// run replacing the previous snapshot. `None` when not watching; any key
    /// stops it.
    pub watch: Option<WatchState>,
    /// The most recently submitted query, so `:watch` with no query reuses it.
    pub last_query: Option<String>,
    /// A one-shot `:o` redirect armed for the next submitted query (issue 12).
    pub redirect: Option<(crate::OutputFormat, std::path::PathBuf)>,
    /// The Named-query store (issue 13): saved templates recalled into the editor
    /// by `:load`. Shared spine with the REPL; the pure reducer mutates it and the
    /// edge persists via [`Effect::PersistQueries`](super::Effect::PersistQueries).
    pub queries: NamedQueries,
    /// The active highlight palette (issue 14): the built-in theme named by the
    /// `theme` Setting with the config `[theme]` overrides applied. Recomputed by
    /// `:set theme`. Drives editor highlighting via [`super::highlight`].
    pub palette: Palette,
    /// The resolved gesture→chord bindings (issue 14): defaults plus config
    /// `[keys]` rebindings. The reducer looks a key press up here.
    pub keys: KeyBindings,
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
            history: Vec::new(),
            view: 0,
            spinner: 0,
            detail: None,
            detail_scroll: 0,
            export: None,
            search: None,
            help: false,
            help_scroll: 0,
            completion: None,
            completer: Completer::with_static_vocabulary(),
            schema: None,
            drawer: None,
            buffers: vec![Buffer::default()],
            active: 0,
            tabbar_area: Rect::default(),
            viewport_rows: 0,
            editor_area: Rect::default(),
            results_area: Rect::default(),
            params: BTreeMap::new(),
            next_id: 0,
            running_statement: None,
            running_buffer: None,
            history_entries: Vec::new(),
            recall_index: None,
            recall_saved: None,
            status: StatusLine::default(),
            color,
            settings: config.settings.clone(),
            read_only: config.read_only,
            tx: TransactionState::Auto,
            profile: config.profile.clone(),
            endpoint: config.endpoint.clone(),
            database: None,
            source_halt: false,
            watch: None,
            last_query: None,
            redirect: None,
            queries: config.queries.clone(),
            palette: config.palette,
            keys: config.keys.clone(),
            config,
        }
    }

    /// The result currently shown (selected by [`view`](Self::view)), if any.
    pub fn shown(&self) -> Option<&CurrentResult> {
        self.history.get(self.view)
    }

    /// Mutable access to the shown result (for table navigation of it).
    pub fn shown_mut(&mut self) -> Option<&mut CurrentResult> {
        self.history.get_mut(self.view)
    }

    /// The result history and view index of the Buffer that owns the in-flight
    /// query (issue 03): the live top-level fields when that Buffer is active,
    /// otherwise its parked [`Buffer`]. `None` when no query is in flight. Lets a
    /// query's Records stream into their origin Buffer regardless of which Buffer
    /// is shown, so the running query *belongs* to its Buffer (CONTEXT.md).
    pub fn running_target(&mut self) -> Option<(&mut Vec<CurrentResult>, &mut usize)> {
        let idx = self.running_buffer?;
        if idx == self.active {
            Some((&mut self.history, &mut self.view))
        } else {
            self.buffers.get_mut(idx).map(|b| (&mut b.history, &mut b.view))
        }
    }

    /// The number of open Buffers (issue 18); always ≥1.
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Park the active Buffer's live state into `buffers[active]` and check out
    /// `target`'s state into the live fields, making it active. A no-op when
    /// `target` is already active. The transient command-history list and all
    /// session-global state are untouched — only the per-Buffer state moves.
    fn checkout(&mut self, target: usize) {
        if target == self.active || target >= self.buffers.len() {
            return;
        }
        self.buffers[self.active] = Buffer {
            editor: std::mem::take(&mut self.editor),
            history: std::mem::take(&mut self.history),
            view: self.view,
            recall_index: self.recall_index.take(),
            recall_saved: self.recall_saved.take(),
        };
        let incoming = std::mem::take(&mut self.buffers[target]);
        self.editor = incoming.editor;
        self.history = incoming.history;
        self.view = incoming.view;
        self.recall_index = incoming.recall_index;
        self.recall_saved = incoming.recall_saved;
        self.active = target;
    }

    /// Open a fresh Buffer after the current one and make it active (issue 18).
    pub fn new_buffer(&mut self) {
        // Park the active Buffer, append an empty one, and check it out.
        let prior_active = self.active;
        self.buffers[self.active] = self.take_active();
        self.buffers.insert(self.active + 1, Buffer::default());
        self.active += 1;
        // A live query's origin Buffer (issue 03) keeps pointing at the same
        // Buffer across the insertion: indices after the insertion point shift up.
        if let Some(idx) = self.running_buffer {
            if idx > prior_active {
                self.running_buffer = Some(idx + 1);
            }
        }
        let incoming = std::mem::take(&mut self.buffers[self.active]);
        self.install_active(incoming);
    }

    /// Close the active Buffer (issue 18). Closing the last is a no-op (there is
    /// always ≥1 Buffer); otherwise the neighbour becomes active.
    pub fn close_buffer(&mut self) {
        if self.buffers.len() == 1 {
            return;
        }
        self.buffers.remove(self.active);
        if self.active >= self.buffers.len() {
            self.active = self.buffers.len() - 1;
        }
        let incoming = std::mem::take(&mut self.buffers[self.active]);
        self.install_active(incoming);
    }

    /// Switch directly to Buffer `index` (issue 18), e.g. from a tab-bar click. A
    /// no-op for the active index or an out-of-range one.
    pub fn switch_to(&mut self, index: usize) {
        self.checkout(index);
    }

    /// Switch to the next/previous Buffer, wrapping (issue 18).
    pub fn cycle_buffer(&mut self, forward: bool) {
        let len = self.buffers.len();
        if len <= 1 {
            return;
        }
        let target = if forward {
            (self.active + 1) % len
        } else {
            (self.active + len - 1) % len
        };
        self.checkout(target);
    }

    /// Move the live top-level Buffer state out into an owned [`Buffer`].
    fn take_active(&mut self) -> Buffer {
        Buffer {
            editor: std::mem::take(&mut self.editor),
            history: std::mem::take(&mut self.history),
            view: self.view,
            recall_index: self.recall_index.take(),
            recall_saved: self.recall_saved.take(),
        }
    }

    /// Install an owned [`Buffer`] as the live top-level state.
    fn install_active(&mut self, buffer: Buffer) {
        self.editor = buffer.editor;
        self.history = buffer.history;
        self.view = buffer.view;
        self.recall_index = buffer.recall_index;
        self.recall_saved = buffer.recall_saved;
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
    /// The statement that produced this result (for history + plan detection).
    pub statement: String,
    pub header: Vec<String>,
    pub rows: Vec<Record>,
    /// When `Some`, this is an `EXPLAIN`/`PROFILE` result rendered as an operator
    /// tree instead of a table (slice 14).
    pub plan: Option<Plan>,
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
    /// A fresh result for a query's `statement` and `header`, cursor at the
    /// top-left.
    pub fn new(statement: String, header: Vec<String>) -> Self {
        Self {
            statement,
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

/// A toggleable side drawer. Schema lands in slice 13; parameters (slice 16) and
/// the summary (slice 18) join it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawerKind {
    Schema,
    Params,
    Summary,
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
// Not `Debug`: carries the connect context (ConnectOptions has credentials, and
// is not `Debug` — nor should secrets be printed).
#[derive(Clone)]
pub struct WorkbenchConfig {
    /// The editor pane's share of the vertical split, in percent.
    pub editor_percent: u16,
    /// The universal newline key, shown in the status hint (slice 01). Ctrl/Shift
    /// +Enter on capable terminals is negotiated in slice 06.
    pub newline_hint: &'static str,
    /// The memory backstop on rows held for one result (see [`DEFAULT_ROW_CAP`]).
    pub row_cap: usize,
    /// Whether `--verbose-execution-info` was set: the summary drawer then shows
    /// the per-query execution info (cost/parse/plan/execute) too (slice 18).
    pub verbose: bool,
    /// The console Settings resolved at startup (default < CLI flag), seeding the
    /// state's live [`Settings`] which `:set` then mutates (issue 01).
    pub settings: Settings,
    /// The active connection profile's name (issue 03), shown in the status bar so
    /// the user always knows which connection they are on. `None` = no profile.
    pub profile: Option<String>,
    /// Whether the Session started read-only (issue 04), seeding the marker state.
    pub read_only: bool,
    /// The Endpoint the Session started on (issue 07), shown in the status bar.
    pub endpoint: String,
    /// The config + current connect options, so `:connect` can resolve a profile
    /// or bare endpoint and re-establish (issue 07).
    pub connect: ConnectContext,
    /// The saved-queries store loaded at startup (issue 13), seeding the state's
    /// live [`NamedQueries`] which `:save`/`:forget` then mutate and persist.
    pub queries: NamedQueries,
    /// The startup highlight palette (issue 14): the built-in theme named by the
    /// `theme` Setting with the config `[theme]` overrides applied.
    pub palette: Palette,
    /// The resolved gesture→chord bindings (issue 14): defaults plus `[keys]`.
    pub keys: KeyBindings,
    /// The config `[theme]` colour overrides, kept so `:set theme` can re-resolve
    /// the palette over a different built-in base at runtime (issue 14).
    pub theme_overrides: BTreeMap<String, String>,
}

/// An active `:watch` (issue 11): which query to re-run and the tick countdown
/// between runs (the render loop ticks at a fixed period).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchState {
    pub query: String,
    pub period_ticks: u32,
    pub remaining: u32,
}

/// What the `:connect` edge needs to resolve a target and re-establish (issue 07).
#[derive(Clone, Default)]
pub struct ConnectContext {
    pub config: Config,
    pub options: ConnectOptions,
}

impl Default for WorkbenchConfig {
    fn default() -> Self {
        Self {
            editor_percent: 40,
            newline_hint: "Alt+Enter",
            row_cap: DEFAULT_ROW_CAP,
            verbose: false,
            settings: Settings::default(),
            profile: None,
            read_only: false,
            endpoint: String::new(),
            connect: ConnectContext::default(),
            queries: NamedQueries::in_memory(),
            palette: builtin_palette("default").expect("default is built in"),
            keys: KeyBindings::default(),
            theme_overrides: BTreeMap::new(),
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

    /// Replace the buffer with `text` (used by history recall, slice 17).
    pub fn set_text(&mut self, text: &str) {
        self.textarea = TextArea::new(text.split('\n').map(String::from).collect());
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

    /// The word being typed under the cursor — the text from the word start (the
    /// REPL's [`word_start`], reused unchanged) to the cursor on the current
    /// line. The completion prefix.
    pub fn word_under_cursor(&self) -> String {
        let (row, col) = self.textarea.cursor();
        let line = &self.textarea.lines()[row];
        let byte = char_to_byte(line, col);
        line[word_start(line, byte)..byte].to_string()
    }

    /// Replace the `prefix_len`-character word before the cursor with `candidate`
    /// (completion insertion): delete the prefix, then insert the candidate.
    pub fn insert_completion(&mut self, prefix_len: usize, candidate: &str) {
        for _ in 0..prefix_len {
            self.textarea.delete_char();
        }
        self.textarea.insert_str(candidate);
    }
}

/// The byte offset of character index `col` in `line` (the cursor's byte
/// position), or the line length when the cursor is at the end.
fn char_to_byte(line: &str, col: usize) -> usize {
    line.char_indices()
        .nth(col)
        .map_or(line.len(), |(byte, _)| byte)
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
