# 01 — Design draft for review: shell, lifecycle & reducer

Status: approved (reviewed 2026-06-10; D1–D4 settled — clear to implement & fan out)

This is the design artifact issue 01 calls for ("HITL … the reducer/effects shape
and the panic-safe lifecycle … warrant a design review before fan-out"). It is
**not yet implemented** — it exists so the State / Event / Effect vocabulary and
the IO lifecycle can be reviewed against all 17 downstream slices before they are
built on top. Binding decision: ADR 0010; constraints: ADR 0002/0004/0005/0008.

The north star is the REPL's `run_loop` (`cli/src/repl.rs`): a pure loop split
from its IO over the `LineSource` / `QueryRunner` seams, driven in tests by
scripted fakes. The workbench is the same shape, async: a pure reducer driven by
events, with the ratatui draw and the async Session execution as thin edges.

---

## 1. Module layout

```
cli/src/
  frontend.rs            # Frontend-selection resolver (pure) — NOT feature-gated
  workbench/
    mod.rs               # pub use; the IO entry `run(session, cfg) -> Result<()>`
    state.rs             # WorkbenchState + sub-states            (pure)
    event.rs             # Event, Key (Frontend-neutral)          (pure)
    effect.rs            # Effect                                 (pure)
    update.rs            # the reducer: WorkbenchState::update     (pure)
    draw.rs              # ratatui draw(frame, &state)            (thin IO edge)
    terminal.rs          # panic-safe raw-mode / alt-screen guard (thin IO edge)
    exec.rs              # async execution edge: Effect -> Session, emits Events
```

The four **pure** modules (state/event/effect/update) carry the behaviour and the
bulk of the tests. They depend only on `mgconsole_core` and std — **not on
crossterm or ratatui** — exactly as `repl.rs`'s `Line`/`MetaCommand` are
terminal-neutral. The three **edge** modules are the analogue of `main`'s
rustyline `Helper` + `SessionRunner`: small, hard to unit-test, kept thin.

The whole `workbench` module is gated behind the `tui` feature (D2); since `tui`
is on by default, the reducer's tests run under a plain `cargo test`. The pure
modules avoid crossterm/ratatui types so they *could* be ungated later if a
lean build ever needed them, but there is no reason to today. (`EditorState`
wraps `tui_textarea::TextArea`, so `state.rs` does touch the widget crate — see
D1; its headless `input()` keeps reducer tests terminal-free regardless.)

---

## 2. Frontend selection resolver (pure)  — AC 1

Mirrors `ColorChoice::resolve`: a pure function, unit-tested, no IO.

```rust
// cli/src/frontend.rs  (sketch)
pub enum Frontend { Piped, Repl, Workbench }

/// Choose the Frontend from the three signals. The piped path (automation) is
/// selected first and bypasses both interactive Frontends, exactly as today.
/// Within an interactive terminal, the workbench wins unless `--plain` is set or
/// the terminal cannot host it. `supports_tui` folds in the compile-time feature:
/// the caller passes `cfg!(feature = "tui") && terminal_is_capable`.
pub fn select_frontend(plain: bool, stdin_is_tty: bool, supports_tui: bool) -> Frontend {
    if !stdin_is_tty { Frontend::Piped }
    else if plain || !supports_tui { Frontend::Repl }
    else { Frontend::Workbench }
}
```

`main` calls this once after parsing flags and resolving colour; `Piped` routes to
today's import/pipe path, `Repl` to today's `run_loop` wiring, `Workbench` to
`workbench::run`. `--color`/`NO_COLOR` is resolved **separately** (existing
`ColorChoice::resolve`) and passed into whichever Frontend runs (AC 4).

---

## 3. The pure state

Defined in full now so downstream slices add *fields*, never reshape the type.
Each field is annotated with the slice that first populates it; issue 01 ships the
**shell** subset (editor, focus, status, color, config, should_quit) and leaves the
rest at their empty defaults.

```rust
// cli/src/workbench/state.rs  (sketch)
pub struct WorkbenchState {
    pub editor:     EditorState,              // 01/05/06/11 — multiline buffer, cursor, selection, undo
    pub focus:      Focus,                    // 01        — Editor | Results | Drawer(DrawerKind)
    pub run:        RunState,                 // 02/07     — Idle | Running { id, started, rows }
    pub pending:    VecDeque<String>,         // 02/10     — statements left to run from one submit
    pub history:    ResultHistory,            // 02/03/10  — stack of ResultEntry + view cursor
    pub completion: CompletionState,          // 11/12     — popup open, candidates, selection
    pub schema:     Option<Schema>,           // 12/13     — None == server feature off (silent degrade)
    pub params:     BTreeMap<String, Value>,  // 16        — reused :param store
    pub drawers:    Drawers,                   // 13/16/18  — schema | params | summary | detail visibility
    pub status:     StatusLine,               // 01/07/18  — message, running indicator, "result N of M"
    pub color:      bool,                      // 01        — resolved colour setting
    pub config:     WorkbenchConfig,           // 01        — row-cap backstop, newline key, kbd-protocol caps
    pub next_id:    u64,                       // 02        — monotonic query id (see Effect/Event)
    pub should_quit: bool,                     // 01
}

pub enum Focus { Editor, Results, Drawer(DrawerKind) }
pub enum DrawerKind { Schema, Params, Summary, CellDetail }

pub enum RunState {
    Idle,
    Running { id: u64, rows_so_far: usize },   // elapsed is wall-clock at the edge; reducer holds the count
}

/// One statement's result, kept so history can revisit it (slice 10).
pub struct ResultEntry {
    pub statement: String,
    pub view:      ResultView,
    pub summary:   Option<Summary>,
    pub partial:   bool,                        // labelled after a cancel (slice 07)
}

/// The pane is polymorphic (ADR 0010): a table for ordinary queries, a tree for
/// EXPLAIN/PROFILE (slice 14/15).
pub enum ResultView { Table(TableState), Plan(PlanTree) }

pub struct TableState {
    pub header:    Vec<String>,
    pub rows:      Vec<Record>,                 // raw Values retained for cell-expand (08) & export (09)
    pub scroll:    usize,
    pub selected:  (usize, usize),              // (row, col) for selection / expand
    // cell text is produced lazily on draw via the Core per-Value renderer
}
```

`rows: Vec<Record>` (not pre-rendered strings) is deliberate: cell-expand (08) and
export (09) need the live `Value`s, and the Core's per-Value renderer produces cell
text on draw for only the visible window (03). The `next_id`/`Running { id }` pair
keeps cancellation pure — see §5.

---

## 4. Events and Effects — the reviewable contract

### Events (into the reducer)

```rust
// cli/src/workbench/event.rs  (sketch)
pub enum Event {
    // --- input, translated from crossterm at the edge into a neutral Key ---
    Key(Key),
    Resize(u16, u16),
    Tick,                                       // ~drives the running spinner/elapsed (07)

    // --- query lifecycle, from exec.rs over a channel (02) ---
    QueryStarted   { id: u64, header: Vec<String>, is_plan: bool },
    RecordArrived  { id: u64, record: Record },
    QueryCompleted { id: u64, summary: Summary },
    QueryFailed    { id: u64, error: Error },
    QueryCancelled { id: u64 },

    // --- schema lifecycle (12/13) ---
    SchemaLoaded(Option<Schema>),               // None == feature off → silent static-only
    // --- export outcome (09) ---
    ExportFinished(Result<PathBuf, String>),
}

/// Frontend-neutral key, the analogue of repl::Line. The crossterm adapter maps
/// crossterm::KeyEvent → Key so the reducer never depends on crossterm.
pub struct Key { pub code: KeyCode, pub mods: Mods }
pub enum KeyCode { Char(char), Enter, Esc, Backspace, Tab, Up, Down, Left, Right, /* … */ }
```

The `id` stamped on every lifecycle event is the guard that realises the
one-live-result rule purely (§5): an event whose `id` is not the current
`Running { id }` is a stale straggler from a cancelled query and is dropped.

### Effects (out of the reducer)

```rust
// cli/src/workbench/effect.rs  (sketch)
pub enum Effect {
    RunQuery { id: u64, query: String, params: BTreeMap<String, Value> },  // 02
    Cancel   { id: u64 },                                                  // 07
    FetchSchema,                                                           // 12/13
    Export   { format: OutputFormat, path: PathBuf, rows: Vec<Record>, header: Vec<String> }, // 09
    AppendHistory(String),                                                 // 17 (persisted file history)
    Quit,                                                                  // 01
}
```

Editor cursor/selection/undo mutations are **not** effects and **not** reducer
events: they are handled by the editor widget at the edge (see Q1). The reducer
intercepts only *gesture* keys — Enter (submit), the newline key, the completion
trigger, Ctrl-C, Esc/Ctrl-D, drawer toggles — and reads the editor's buffer string
when it submits.

### The reducer

```rust
// cli/src/workbench/update.rs  (sketch)
impl WorkbenchState {
    pub fn update(&mut self, event: Event) -> Vec<Effect> { … }
}
```

Worked transitions (the ones the human most needs to bless):

- **Submit (Enter, run idle).** Split the editor buffer with the existing
  `QueryAssembler` into `Vec<String>`. Push all-but-first onto `self.pending`,
  set `RunState::Running { id: self.next_id, rows_so_far: 0 }`, return
  `vec![Effect::RunQuery { id, query: first, params }, Effect::AppendHistory(buffer)]`.
  A buffer with no `;` is one statement (PRD: submission is decoupled from the
  REPL's `;`-completeness rule).
- **Submit while Running.** Return `vec![]` and set the status to
  `"session busy — cancel first"` (AC 5, ADR 0005 one-live-result). No effect.
- **RecordArrived** (id matches): append to the current `TableState.rows`,
  `rows_so_far += 1`. (id stale: drop.)
- **QueryCompleted** (id matches): finalise the entry with its `Summary`, push to
  `history`; if `pending` non-empty, pop the next and emit the next `RunQuery`
  (sequential multi-statement on one Session); else `RunState::Idle`.
- **Ctrl-C while Running**: `RunState::Idle`, mark the current entry `partial`
  with its `rows_so_far`, status `"cancelled — N rows (partial)"`, return
  `vec![Effect::Cancel { id }]` (07).
- **QueryFailed**: status shows the error, `RunState::Idle`, session survives
  (the next submit runs) — AC 4 / ADR 0005.
- **Esc / Ctrl-D / `:quit`**: `should_quit = true`, return `vec![Effect::Quit]`.

This is the seam slice 02+ test like `run_loop`: feed an `Event` sequence, assert
the resulting state and the `Vec<Effect>`, with no terminal and no database.

---

## 5. The async execution edge (exec.rs) — thin

The analogue of `SessionRunner`, made async. Issue 01 ships the **loop skeleton**
and stubs execution (Enter produces no rows yet); slice 02 fills in `perform`.

```rust
// cli/src/workbench/mod.rs  (sketch of the IO loop)
pub async fn run(session: Session, cfg: WorkbenchConfig, color: bool) -> Result<()> {
    let _guard = TerminalGuard::enter()?;          // raw mode + alt screen; restores on drop & panic (§6)
    let mut term = ratatui::Terminal::new(/* crossterm backend */)?;
    let session = Arc::new(Mutex::new(session));   // shared with the run task; see below
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let mut input = crossterm::event::EventStream::new();
    let mut state = WorkbenchState::new(cfg, color);

    loop {
        term.draw(|f| draw::draw(f, &state))?;
        let event = tokio::select! {
            Some(Ok(ev)) = input.next() => translate(ev),   // crossterm -> Event::Key/Resize
            Some(ev) = rx.recv()        => ev,              // lifecycle/schema/export events
            () = tick()                 => Event::Tick,
        };
        for effect in state.update(event) {
            perform(effect, &session, &tx).await;           // spawns/aborts tasks, runs writers, etc.
        }
        if state.should_quit { break; }
    }
    Ok(())
}
```

**Cancellation, purely + concretely.** `Effect::RunQuery` spawns a task that locks
the shared `Arc<Mutex<Session>>`, calls `run_with_params`, then streams
`RecordStream::next()` → `Event::RecordArrived`, ending in `QueryCompleted`/
`QueryFailed`; the edge keeps its `JoinHandle` keyed by `id`. `Effect::Cancel { id }`
aborts that handle (dropping the lock and the `RecordStream`) and then issues the
Session's RESET recovery — ADR 0005's existing machinery: a dropped-before-drain
stream is exactly what `enforce_one_live_result` resets. The reducer never sees a
task or a lock; it only emits `Cancel { id }` and trusts stale-`id` events to be
dropped. `Arc<Mutex<Session>>` (the `conn` is already `Arc<Mutex>` inside) is what
lets the aborted task release the connection cleanly while the next query waits.

(Q4 asks whether to confirm this spawn-task-+-channel model vs. a single-loop
`select!` directly over the live stream — both are non-blocking and cancellable;
this matches slice 02's "cancellable task … over a channel" wording.)

---

## 6. Panic-safe terminal lifecycle — AC 3

```rust
// cli/src/workbench/terminal.rs  (sketch)
pub struct TerminalGuard;
impl TerminalGuard {
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;   // + keyboard-enhancement flags negotiated in slice 06
        install_panic_hook();                          // chains the existing hook, restores first
        Ok(Self)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) { let _ = execute!(stdout(), LeaveAlternateScreen); let _ = disable_raw_mode(); }
}
```

The panic hook restores the terminal **before** the default hook prints the
message (otherwise the backtrace lands on the alternate screen and the shell is
left in raw mode). `Drop` covers the normal-exit and unwind paths; the hook covers
printing during unwind. Together: no corrupted shell on quit **or** panic (the
standard ratatui pattern). Tested by a smoke test that the hook is installed and
restores; the lifecycle itself is inherently an IO edge.

---

## 7. Layout, keys, and what 01 actually ships

- **Layout:** vertical split — editor pane (top), results pane (middle), one-line
  status bar (bottom). The split ratio lives in `config` with a sensible default
  and a resize keybind. Drawers/overlays (schema, params, summary, cell-detail)
  are toggled, not always-on (their slices add them).
- **Keys in slice 01:** Enter = submit (wired to the reducer; execution is a no-op
  stub until slice 02); the **universal newline key** inserts a newline and is
  shown in the status hint; Esc / Ctrl-D / `:quit` exit. Shift/Ctrl+Enter via the
  keyboard protocol is slice 06; slice 01 only guarantees Enter-submits and the
  universal key works everywhere.
- **Cargo feature:** a `tui` feature in `cli/Cargo.toml` pulls in `ratatui` +
  `crossterm` + `tui-textarea` (D1), and is in the default set (D2).
  `#[cfg(feature = "tui")] pub mod workbench;`. `frontend.rs` compiles always and
  returns `Repl`/`Piped` when `tui` is absent (`--no-default-features`).

**Slice 01 deliverable:** the resolver (§2, tested), the full pure State/Event/
Effect/`update` types (§3–4) with shell-level transitions tested by scripted event
sequences (no terminal, no DB), the panic-safe lifecycle (§6), the draw skeleton,
and the loop skeleton (§5) with execution stubbed. Everything 02–18 then fills in
fields and transitions without reshaping the contract.

---

## Resolved decisions (reviewed 2026-06-10)

- **D1 — Editor: reuse `tui-textarea`.** It provides multiline editing, free
  cursor movement, selection, and undo, and runs **headlessly** (its `input()`
  mutates state with no terminal), so reducer tests stay terminal-free. The
  reducer owns only the gesture keys (Enter/newline/completion/Ctrl-C/Esc/drawer
  toggles) and reads the buffer string on submit; ordinary editing keys are fed
  to the `TextArea` at the edge. `EditorState` wraps a `tui_textarea::TextArea`.
  The dependency lives under the `tui` feature. Affects slices 05/06/11.
- **D2 — `tui` is a default Cargo feature (on by default).** `cargo build`
  includes the workbench and it is the default experience (ADR 0010). AC 6's
  "the default binary builds without them" is satisfied via
  `cargo build --no-default-features` (a `default = ["tui"]` feature set, so the
  ratatui/crossterm/tui-textarea deps are all behind `tui` and a
  `--no-default-features` build is REPL-only and compiles without them).
  `frontend.rs` still compiles in both configurations and returns `Repl`/`Piped`
  when `tui` is absent.
- **D3 — Reducer signature is `fn update(&mut self, ev) -> Vec<Effect>`** (mutate
  in place, not the literal Elm `(Self, Vec<Effect>)`), avoiding a clone of the
  `Vec<Record>` on every keystroke. Still pure: no IO, deterministic,
  event-in/state-out testable.
- **D4 — Execution model: spawned task + `Arc<Mutex<Session>>` + abort-to-cancel**
  (§5), matching slice 02's "cancellable task … over a channel." The reducer
  emits `RunQuery`/`Cancel { id }` and stays oblivious to the task and the lock.
