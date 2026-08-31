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
use super::effect::TxOp;
use super::event::{Key, KeyCode};
use super::plan::Plan;
use super::schema::Schema;

/// The disposition of a Transaction episode (issue 04 / CONTEXT.md): a tagged
/// Result-history entry starts [`Open`] and is resolved retroactively to
/// [`Committed`] or [`RolledBack`] when the episode ends.
///
/// [`Open`]: Self::Open
/// [`Committed`]: Self::Committed
/// [`RolledBack`]: Self::RolledBack
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Open,
    Committed,
    RolledBack,
}

impl Disposition {
    /// The bracketed label shown in the results-pane header (issue 04).
    pub fn label(self) -> &'static str {
        match self {
            Disposition::Open => "open",
            Disposition::Committed => "committed",
            Disposition::RolledBack => "rolled back",
        }
    }
}

/// A Result-history entry's place in a Transaction episode (issue 04): the episode
/// number, the statement's ordinal within it, and the episode's disposition. A
/// query run inside `:begin`…`:commit`/`:rollback` carries one of these so it is
/// reviewable as *what it was* — the Nth statement of an episode that was
/// ultimately committed or undone. Autocommit entries carry `None` instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionTag {
    pub episode: u32,
    pub ordinal: u32,
    pub disposition: Disposition,
}

impl TransactionTag {
    /// The header label, e.g. `tx 2 · stmt 3 [open]` (issue 04).
    pub fn label(self) -> String {
        format!(
            "tx {} · stmt {} [{}]",
            self.episode,
            self.ordinal,
            self.disposition.label()
        )
    }
}

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

/// The modal Command line (ADR 0017): a one-line prompt for the typed
/// `:`-vocabulary (Meta-commands and Workbench commands). It opens on `:` while
/// the Buffer editor is empty/whitespace-only, or on `Ctrl+G` anywhere, runs its
/// content on Enter, and dismisses on Esc — returning focus to [`prior_focus`].
/// It is *not* part of the focus cycle, so the editor stays Cypher-only.
///
/// [`content`] always carries the leading `:` (seeded when the prompt opens), so
/// it parses through the same [`meta_command`](crate::repl::meta_command) path the
/// REPL prompt uses.
///
/// [`content`]: Self::content
/// [`prior_focus`]: Self::prior_focus
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    /// The line being typed, including its leading `:`.
    pub content: String,
    /// The pane focus to restore when the command line closes.
    pub prior_focus: Focus,
    /// The open inline completion menu (issue 02): the matching `:command` names,
    /// the selected one, and the content prefix they complete onto. `None` when no
    /// `Tab` menu is active; cleared by any edit or recall.
    pub completion: Option<CommandCompletion>,
    /// The recall position into the session command history (issue 02), or `None`
    /// while editing the live line. Distinct from the editor's Cypher query recall.
    pub recall_index: Option<usize>,
    /// The live line saved when recall began, restored on stepping past the newest.
    pub recall_saved: Option<String>,
}

impl CommandLine {
    /// A freshly-opened Command line: the `:` prompt seeded, remembering the focus
    /// to restore, with no completion menu or recall in progress.
    pub fn new(prior_focus: Focus) -> Self {
        Self {
            content: ":".to_string(),
            prior_focus,
            completion: None,
            recall_index: None,
            recall_saved: None,
        }
    }
}

/// The inline `Tab`-completion menu for the modal Command line (issue 02): the
/// matching `:command` names, which one is shown, and the content prefix the
/// candidate is appended to (everything before the completed word, e.g. `:`). A
/// repeated `Tab` cycles `selected`; an edit drops the whole menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandCompletion {
    pub candidates: Vec<String>,
    pub selected: usize,
    pub base: String,
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

/// The suspended Workbench *view* parked by the dispatch loop across a Frontend
/// switch (issue 04, ADR 0019): every Buffer (editor text + per-Buffer Result
/// history) and the active index, so a Workbench → REPL → Workbench round-trip
/// restores the tabs and results you left rather than rebuilding one fresh Buffer.
///
/// It is deliberately *only* the Workbench-local view — no session state. read-only,
/// the active Database, params, Settings, and the transaction live in the Session
/// bundle the loop owns and are read live on every Workbench entry, so reattaching a
/// parked view onto the current Session needs no per-field reconciliation: the
/// Buffers are pure editor + results, and everything session-scoped is already fresh.
pub struct WorkbenchView {
    pub buffers: Vec<Buffer>,
    pub active: usize,
}

/// The maximum width (in characters) of an auto-derived tab title (issue 07),
/// before the `…` truncation marker; the tab cell is this plus side padding.
pub const MAX_TAB_TITLE: usize = 12;

/// One rendered Buffer tab in the windowed tab bar (issue 07): which Buffer it is,
/// its absolute screen column and cell width, and its derived title. The draw
/// renders these and the reducer hit-tests a click against their column ranges, so
/// titling, windowing, and click-mapping share one layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabHit {
    pub index: usize,
    pub col: u16,
    pub width: u16,
    pub title: String,
}

/// The laid-out tab bar for one frame (issue 07): the visible tabs and whether
/// there are more Buffers off either edge (so the draw shows overflow markers).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TabBar {
    pub tabs: Vec<TabHit>,
    pub left_more: bool,
    pub right_more: bool,
}

/// The one input-capturing overlay that is open, if any. At most one is ever open,
/// so the six overlays are one sum type rather than six mutually-exclusive
/// `Option` fields: "two overlays open" is then unrepresentable, and the key
/// dispatch, the mouse-ignore, and the draw all consult the single
/// [`modal`](WorkbenchState::modal) field instead of six checks that can drift
/// apart. The scroll offsets fold into the variants that have them.
///
/// `drawer` (a side panel) and `watch` (a "press any key to stop" mode) are *not*
/// modals — they coexist with the panes — so they stay separate.
#[derive(Debug)]
pub enum Modal {
    /// The `:help` keybinding/command overlay, scrolled by `scroll`.
    Help { scroll: u16 },
    /// The cell-detail overlay showing one Value in full (slice 08), scrolled.
    Detail { value: Value, scroll: u16 },
    /// The export prompt (slice 09): pick a format, type a destination path.
    Export(ExportPrompt),
    /// In-result search over the shown result (issue 16).
    Search(SearchState),
    /// The completion popup for the word under the editor cursor (slice 11).
    Completion(Completion),
    /// The modal Command line for the typed `:`-vocabulary (ADR 0017).
    CommandLine(CommandLine),
}

/// The lightweight tag of a [`Modal`] (which overlay), for `Copy` dispatch without
/// borrowing the payload — the reducer matches this to route a key to the right
/// handler, then re-borrows the payload inside that handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    Help,
    Detail,
    Export,
    Search,
    Completion,
    CommandLine,
}

impl Modal {
    /// Which overlay this is, without borrowing its payload.
    pub fn kind(&self) -> ModalKind {
        match self {
            Modal::Help { .. } => ModalKind::Help,
            Modal::Detail { .. } => ModalKind::Detail,
            Modal::Export(_) => ModalKind::Export,
            Modal::Search(_) => ModalKind::Search,
            Modal::Completion(_) => ModalKind::Completion,
            Modal::CommandLine(_) => ModalKind::CommandLine,
        }
    }
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
    /// The one open input-capturing overlay, if any (the [`Modal`]): cell-detail,
    /// export prompt, in-result search, completion popup, modal Command line, or the
    /// `:help` overlay. At most one is open at a time, so they are one sum type
    /// rather than six independent fields — the key cascade, the mouse-ignore, and
    /// the draw all read this one field, and "two overlays open" is unrepresentable.
    /// The side `drawer` and the `:watch` mode are *not* modals (they coexist).
    pub modal: Option<Modal>,
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
    /// The visible tabs and their screen column ranges from the last draw (issue
    /// 07), cached so a tab-bar click maps to the right Buffer under windowing and
    /// variable title widths. The draw is the only writer; the reducer reads.
    pub tab_hits: Vec<TabHit>,
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
    /// The session command history (issue 02): past `:`-commands run from the modal
    /// Command line, oldest→newest, for `Up`/`Down` recall there. In-memory and
    /// session-scoped — kept distinct from the Cypher [`history_entries`] that
    /// `Ctrl+Up/Down` recalls into the editor.
    ///
    /// [`history_entries`]: Self::history_entries
    pub command_history: Vec<String>,
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
    /// Whether terminal mouse capture is on (issue 04). Capture is on by default
    /// (the Workbench's mouse gestures); `:set mouse off` releases it so native
    /// click-drag selection works as the documented escape hatch. The reducer
    /// tracks it for `:set` listing; the edge applies the actual capture toggle.
    pub mouse: bool,
    /// The explicit-transaction state (issue 05), mirrored from the Session by the
    /// `TransactionApplied` event. Drives the `[tx]`/`[tx failed]` status marker.
    pub tx: TransactionState,
    /// The current Transaction episode number (issue 04): incremented when a
    /// `:begin` opens a transaction, so each episode is distinct. Session-scoped —
    /// shared across all Buffers, since they share the one open Transaction.
    pub tx_episode: u32,
    /// The per-statement ordinal within the current episode (issue 04): reset on
    /// `:begin`, incremented on each submission made while the Transaction is Open.
    /// Continues across Buffers, so statements from several Buffers share one
    /// ordinal sequence within the episode.
    pub tx_ordinal: u32,
    /// The explicit-transaction op awaiting its `TransactionApplied` result (issue
    /// 04): set when `:begin`/`:commit`/`:rollback` is issued, consumed when the
    /// outcome arrives so the episode is opened or resolved by the *actual* result
    /// (a poisoned `:commit` that the server rejects does not resolve the episode).
    pub pending_tx_op: Option<TxOp>,
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
    /// The Named-query store (issue 23, ADR 0020): a directory of `<name>.cypher`
    /// files mirrored in memory, recalled into the editor by `:load`. Shared spine
    /// with the REPL; the pure reducer mutates the mirror and the edge writes/deletes
    /// the one changed file via [`Effect::PersistQuery`](super::Effect::PersistQuery).
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
            // Seed the active Buffer's editor with any query text carried in from a
            // Frontend switch (issue 03); empty on a fresh launch yields a blank editor.
            editor: if config.initial_editor.is_empty() {
                EditorState::new()
            } else {
                EditorState::with_text(&config.initial_editor)
            },
            focus: Focus::Editor,
            run: RunState::Idle,
            pending: VecDeque::new(),
            history: Vec::new(),
            view: 0,
            spinner: 0,
            modal: None,
            completer: Completer::with_static_vocabulary(),
            schema: None,
            drawer: None,
            buffers: vec![Buffer::default()],
            active: 0,
            tabbar_area: Rect::default(),
            tab_hits: Vec::new(),
            viewport_rows: 0,
            editor_area: Rect::default(),
            results_area: Rect::default(),
            params: config.params.clone(),
            next_id: 0,
            running_buffer: None,
            history_entries: Vec::new(),
            command_history: Vec::new(),
            recall_index: None,
            recall_saved: None,
            status: StatusLine::default(),
            color,
            settings: config.settings.clone(),
            read_only: config.read_only,
            mouse: true,
            // Seeded from the live Session (issue 04) so an explicit transaction
            // opened before a Frontend switch shows its `[tx]` marker on re-entry.
            tx: config.tx,
            tx_episode: 0,
            tx_ordinal: 0,
            pending_tx_op: None,
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

    /// Which [`Modal`] is open, if any — the `Copy` tag the key dispatch matches on
    /// before re-borrowing the payload inside the chosen handler.
    pub fn modal_kind(&self) -> Option<ModalKind> {
        self.modal.as_ref().map(Modal::kind)
    }

    /// The open in-result search state (issue 16), if search is the open Modal —
    /// a read/write view into the one [`modal`](Self::modal) field, never a second
    /// place that could disagree with it.
    pub fn search(&self) -> Option<&SearchState> {
        if let Some(Modal::Search(s)) = &self.modal { Some(s) } else { None }
    }
    pub fn search_mut(&mut self) -> Option<&mut SearchState> {
        if let Some(Modal::Search(s)) = &mut self.modal { Some(s) } else { None }
    }
    /// The open completion popup (slice 11), if it is the open Modal.
    pub fn completion(&self) -> Option<&Completion> {
        if let Some(Modal::Completion(c)) = &self.modal { Some(c) } else { None }
    }
    pub fn completion_mut(&mut self) -> Option<&mut Completion> {
        if let Some(Modal::Completion(c)) = &mut self.modal { Some(c) } else { None }
    }
    /// The open Command line (ADR 0017), if it is the open Modal.
    pub fn command_line(&self) -> Option<&CommandLine> {
        if let Some(Modal::CommandLine(c)) = &self.modal { Some(c) } else { None }
    }
    pub fn command_line_mut(&mut self) -> Option<&mut CommandLine> {
        if let Some(Modal::CommandLine(c)) = &mut self.modal { Some(c) } else { None }
    }
    /// The cell-detail overlay's Value, if detail is the open Modal (slice 08).
    pub fn detail(&self) -> Option<&Value> {
        if let Some(Modal::Detail { value, .. }) = &self.modal { Some(value) } else { None }
    }
    /// The cell-detail overlay's scroll offset, if detail is the open Modal.
    pub fn detail_scroll(&self) -> Option<u16> {
        if let Some(Modal::Detail { scroll, .. }) = &self.modal { Some(*scroll) } else { None }
    }
    /// The `:help` overlay's scroll offset, if help is the open Modal.
    pub fn help_scroll(&self) -> Option<u16> {
        if let Some(Modal::Help { scroll }) = &self.modal { Some(*scroll) } else { None }
    }
    /// The export prompt, if it is the open Modal (slice 09).
    pub fn export(&self) -> Option<&ExportPrompt> {
        if let Some(Modal::Export(e)) = &self.modal { Some(e) } else { None }
    }
    /// Whether the `:help` overlay is the open Modal.
    pub fn help_open(&self) -> bool {
        matches!(self.modal, Some(Modal::Help { .. }))
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

    /// The auto-derived title for Buffer `index` (issue 07): a short snippet of its
    /// content — the originating query of its shown Result-history entry, falling
    /// back to the first non-empty editor line, then to the Buffer number — with a
    /// leading `•` when that Buffer owns the in-flight query. Whitespace is
    /// collapsed and the text truncated to [`MAX_TAB_TITLE`] with a trailing `…`.
    pub fn buffer_title(&self, index: usize) -> String {
        // The active Buffer's state is top-level; the others are parked.
        let (editor, history, view) = if index == self.active {
            (&self.editor, &self.history, self.view)
        } else {
            let buffer = &self.buffers[index];
            (&buffer.editor, &buffer.history, buffer.view)
        };
        let from_history = history
            .get(view)
            .map(|result| result.statement.as_str())
            .filter(|s| !s.trim().is_empty());
        let from_editor = || {
            editor
                .lines()
                .iter()
                .map(|line| line.trim())
                .find(|line| !line.is_empty())
        };
        let raw = from_history.or_else(from_editor);
        let collapsed = raw
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty())
            // A Buffer with neither a result nor typed text falls back to its number.
            .unwrap_or_else(|| (index + 1).to_string());
        let title = if collapsed.chars().count() > MAX_TAB_TITLE {
            let mut s: String = collapsed.chars().take(MAX_TAB_TITLE - 1).collect();
            s.push('…');
            s
        } else {
            collapsed
        };
        let marker = if self.running_buffer == Some(index) { "•" } else { "" };
        format!("{marker}{title}")
    }

    /// Lay out the windowed tab bar for `area` (issue 07): derive each Buffer's
    /// title, size its cell (title + one-space padding each side), and window the
    /// bar so the active tab stays visible, leaving a column for an overflow marker
    /// on each side that has more Buffers. Pure — the draw renders the result and
    /// caches the tabs for click hit-testing, so titling/windowing/click agree.
    pub fn layout_tabs(&self, area: Rect) -> TabBar {
        let count = self.buffer_count();
        let titles: Vec<String> = (0..count).map(|i| self.buffer_title(i)).collect();
        let widths: Vec<u16> = titles
            .iter()
            .map(|t| t.chars().count() as u16 + 2)
            .collect();
        let total: u16 = widths.iter().sum();
        // Reserve a column for a marker on each side when the bar overflows.
        let overflow = total > area.width;
        let budget = if overflow { area.width.saturating_sub(2) } else { area.width };
        let (start, end) = window_tabs(&widths, self.active, budget);
        let left_more = start > 0;
        let right_more = end < count;
        let mut tabs = Vec::with_capacity(end - start);
        let mut x = area.x + u16::from(left_more);
        for index in start..end {
            tabs.push(TabHit {
                index,
                col: x,
                width: widths[index],
                title: titles[index].clone(),
            });
            x += widths[index];
        }
        TabBar { tabs, left_more, right_more }
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
        let removed = self.active;
        self.buffers.remove(removed);
        // A live query's origin Buffer (issue 03) keeps pointing at the same Buffer
        // across the removal: indices after the removed one shift down. The caller
        // never closes the owning Buffer without first clearing `running_buffer`
        // (it cancels the query), so the owning index is never the one removed.
        if let Some(idx) = self.running_buffer {
            if idx > removed {
                self.running_buffer = Some(idx - 1);
            }
        }
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

    /// Detach the whole Workbench view to be parked across a Frontend switch (issue
    /// 04): park the active Buffer's live state into its slot, then move out the
    /// Buffer set and active index. The whole-`Vec` analogue of [`checkout`](Self::checkout).
    pub fn detach_view(&mut self) -> WorkbenchView {
        self.buffers[self.active] = self.take_active();
        WorkbenchView {
            buffers: std::mem::take(&mut self.buffers),
            active: self.active,
        }
    }

    /// Reattach a parked view on Workbench re-entry (issue 04): adopt its Buffer set
    /// and active index, then install the active Buffer's state into the live
    /// top-level fields. Session-scoped state is *not* here — it is read live from
    /// the bundle on entry — so this is the whole rebind. `active` is clamped
    /// defensively so an out-of-range index can never panic.
    pub fn install_view(&mut self, view: WorkbenchView) {
        self.buffers = view.buffers;
        self.active = view.active.min(self.buffers.len().saturating_sub(1));
        let incoming = std::mem::take(&mut self.buffers[self.active]);
        self.install_active(incoming);
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

/// Choose the window of tab indices `[start, end)` to show (issue 07): always
/// includes `active`, and grows outward (forward first, then back) while the cells
/// fit `budget`. Returns the whole range when everything fits.
fn window_tabs(widths: &[u16], active: usize, budget: u16) -> (usize, usize) {
    let count = widths.len();
    if count == 0 {
        return (0, 0);
    }
    let active = active.min(count - 1);
    let (mut start, mut end) = (active, active + 1);
    let mut used = widths[active].min(budget);
    loop {
        let mut grew = false;
        if end < count && used + widths[end] <= budget {
            used += widths[end];
            end += 1;
            grew = true;
        }
        if start > 0 && used + widths[start - 1] <= budget {
            used += widths[start - 1];
            start -= 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }
    (start, end)
}

/// Whether a query is in flight, and — while `Running` — everything about the one
/// in-flight statement (one-live-result, ADR 0005). The `id` matches the running
/// query's lifecycle events; events for any other id are stragglers and ignored.
///
/// The per-statement facts live *inside* the `Running` variant so they cannot
/// dangle while `Idle`: `statement` (carried to the result entry, for plan
/// detection), `started` (whether `QueryStarted` has arrived — distinguishes a
/// failure before any record from one after), and `tag` (the Transaction episode
/// stamp, issue 04). The Buffer that owns the query (`running_buffer`) and the rest
/// of the batch (`pending`) live on [`WorkbenchState`] instead — they outlive one
/// statement and belong to the submission, not the in-flight statement.
///
/// Not `Copy` (it owns a `String`): id-only callers go through [`running_id`].
///
/// [`running_id`]: Self::running_id
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Running {
        id: u64,
        statement: String,
        started: bool,
        tag: Option<TransactionTag>,
    },
}

impl RunState {
    /// The id of the query in flight, or `None` when idle — for the many callers
    /// that only need "is a query running, and which id" without the payload.
    pub fn running_id(&self) -> Option<u64> {
        if let RunState::Running { id, .. } = self {
            Some(*id)
        } else {
            None
        }
    }
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
    /// The error this query failed with, if it failed (issue 05): the outcome in
    /// place of Records, so a failed submission is a navigable Result-history
    /// entry carrying its originating [`statement`](Self::statement) and error,
    /// not a status message that scrolls away. `None` for a successful query.
    pub error: Option<String>,
    /// The Transaction episode this entry belongs to (issue 04), or `None` for an
    /// autocommit entry. Stamped at submission while a transaction is Open; its
    /// disposition is resolved retroactively when the episode ends.
    pub tx_tag: Option<TransactionTag>,
    /// Set when this entry has been trimmed from the bounded Result history (issue
    /// 06): its Records were dropped to bound memory, but its lightweight
    /// correlation — statement, summary, error — is kept so it stays navigable as
    /// a record of *what ran*. The draw then shows a "rows no longer held" note in
    /// place of the table.
    pub trimmed: bool,
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

/// A generous cap on the number of *full* Result-history entries held per Buffer
/// (issue 06). Older entries keep their lightweight correlation (query, summary,
/// error) but drop their Records. Where [`DEFAULT_ROW_CAP`] bounds one result,
/// this bounds the count of results so a long session does not grow unbounded.
pub const DEFAULT_HISTORY_CAP: usize = 100;

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
    /// The cap on full Result-history entries held per Buffer (issue 06; see
    /// [`DEFAULT_HISTORY_CAP`]). Older entries keep their correlation but drop
    /// their Records.
    pub history_cap: usize,
    /// Whether `--verbose-execution-info` was set: the summary drawer then shows
    /// the per-query execution info (cost/parse/plan/execute) too (slice 18).
    pub verbose: bool,
    /// The console Settings resolved at startup (default < CLI flag), seeding the
    /// state's live [`Settings`] which `:set` then mutates (issue 01).
    pub settings: Settings,
    /// The `:param` store handed in by the dispatch loop (ADR 0019): empty on a
    /// fresh launch, or the params carried across a Frontend switch. Seeds the
    /// state's live params, which `:param` then mutates.
    pub params: BTreeMap<String, Value>,
    /// The active query editor text carried in from a Frontend switch (issue 03):
    /// the REPL's input line on an up-switch, seeding the one fresh Buffer's editor.
    /// Empty on a fresh launch (a blank editor).
    pub initial_editor: String,
    /// The active connection profile's name (issue 03), shown in the status bar so
    /// the user always knows which connection they are on. `None` = no profile.
    pub profile: Option<String>,
    /// Whether the Session started read-only (issue 04), seeding the marker state.
    pub read_only: bool,
    /// The Session's transaction state on entry (issue 04): seeds the `[tx]` marker
    /// so an explicit transaction opened before a Frontend switch is reflected on
    /// re-entry, not silently shown as autocommit. Read live from the Session by the
    /// dispatch loop (ADR 0019). `Auto` on a fresh launch.
    pub tx: TransactionState,
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
            history_cap: DEFAULT_HISTORY_CAP,
            verbose: false,
            settings: Settings::default(),
            params: BTreeMap::new(),
            initial_editor: String::new(),
            profile: None,
            read_only: false,
            tx: TransactionState::Auto,
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
    /// Snapshot undo stack (issue 09): the buffer text *before* each logical edit,
    /// oldest→newest. One entry per logical operation — a keystroke, a newline, a
    /// completion, or a whole-buffer replace (auto-format / `:load` / recall) — so a
    /// single `undo` restores the previous buffer regardless of how many internal
    /// edits the operation made. A full-buffer snapshot is cheap for query text and
    /// sidesteps tui-textarea's per-internal-edit history (a replace there is two
    /// steps; a select-then-type, two more).
    undo_stack: Vec<String>,
    /// The buffers undone *from*, for redo; cleared whenever a new edit is recorded.
    redo_stack: Vec<String>,
}

impl EditorState {
    /// An empty editor.
    pub fn new() -> Self {
        Self {
            textarea: TextArea::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// An editor pre-seeded with `text` as its baseline (issue 03): the active
    /// query editor text carried in from a Frontend switch. The text is the starting
    /// buffer, not an undoable edit, so the undo stack begins empty — there is no
    /// prior buffer to revert to.
    pub fn with_text(text: &str) -> Self {
        Self {
            textarea: TextArea::new(text.split('\n').map(String::from).collect()),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Record `before` as an undo checkpoint for a logical edit just applied, and
    /// drop the redo history (a new edit forks it). A no-op when nothing changed.
    fn checkpoint(&mut self, before: String) {
        if before != self.buffer() {
            self.undo_stack.push(before);
            self.redo_stack.clear();
        }
    }

    /// Apply an ordinary editing key (character, backspace, arrows, …) to the
    /// buffer. Keys the editor does not act on are ignored. Enter and the newline
    /// key are *not* routed here — the reducer owns those gestures.
    pub fn edit(&mut self, key: Key) {
        if let Some(input) = to_input(key) {
            let before = self.buffer();
            self.textarea.input(input);
            self.checkpoint(before);
        }
    }

    /// Insert a newline at the cursor (the universal newline gesture).
    pub fn insert_newline(&mut self) {
        let before = self.buffer();
        self.textarea.insert_newline();
        self.checkpoint(before);
    }

    /// The whole buffer as one string, physical lines joined by `\n`.
    pub fn buffer(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Discard the buffer back to empty (Ctrl-C abandons typing when idle). This
    /// resets undo — abandoning is itself the "undo" of the typed buffer.
    pub fn clear(&mut self) {
        self.textarea = TextArea::default();
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Replace the whole buffer with `text` as one undoable operation (issue 09).
    /// Used by auto-format, `:load`, and history recall; the previous behaviour
    /// rebuilt the widget, discarding undo so a format could not be reverted and
    /// recalling over unsaved text lost it irrecoverably. Now a single `undo`
    /// (Ctrl+Z) restores the previous buffer.
    pub fn set_text(&mut self, text: &str) {
        let before = self.buffer();
        self.textarea.select_all();
        self.textarea.insert_str(text);
        self.checkpoint(before);
    }

    /// Undo the last logical edit (Ctrl+Z): restore the previous buffer snapshot in
    /// one step. Returns whether anything was undone.
    pub fn undo(&mut self) -> bool {
        if let Some(previous) = self.undo_stack.pop() {
            self.redo_stack.push(self.buffer());
            self.textarea = TextArea::new(previous.split('\n').map(String::from).collect());
            true
        } else {
            false
        }
    }

    /// Redo the last undone edit (Ctrl+Y). Returns whether anything was redone.
    pub fn redo(&mut self) -> bool {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.buffer());
            self.textarea = TextArea::new(next.split('\n').map(String::from).collect());
            true
        } else {
            false
        }
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
        let before = self.buffer();
        for _ in 0..prefix_len {
            self.textarea.delete_char();
        }
        self.textarea.insert_str(candidate);
        self.checkpoint(before);
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
