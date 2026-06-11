# PRD: TUI query workbench Frontend

Status: ready-for-agent

## Problem Statement

People who work with Memgraph from the terminal today drive the console as a
line-based REPL: type a query, wait, read the result, repeat. That model has
hard ceilings the line REPL cannot lift, because it runs each query
synchronously (`block_on`) with the whole process frozen until the last Record
drains:

- A long-running query can only be escaped by killing the shell — there is no
  way to cancel it and keep the session.
- A large result is unreadable: the tabular path caps at a row count and warns,
  so exploring a 50k-row answer interactively is impossible.
- Nothing can happen while a query runs — no live row count, no elapsed timer,
  no scrolling the previous result.
- A node/map/path Value is truncated into a table cell with no way to expand it.
- Completion is limited to the closed keyword/function vocabulary; the names the
  database actually holds (labels, relationship types, property keys) are
  invisible.
- A query plan (`EXPLAIN`/`PROFILE`) is dumped as flat rows, not the tree it is.
- There is no way to revisit a previous query's result.

The Core/Frontend seam (ADR 0002) and streaming-first Records (ADR 0004) were
built precisely so a richer Frontend could lift these ceilings without
rewriting the engine — but no such Frontend exists yet.

## Solution

A full-screen terminal **workbench**: a new interactive Frontend over the
unchanged Core, built on ratatui, that becomes the default interactive
experience (the line REPL is retained behind `--plain`).

From a user's perspective: connect, type Cypher in a real multiline editor, run
it, and explore the answer in a navigable results table that fills in as rows
arrive — cancel it at any time with Ctrl-C and keep the session, scroll a huge
result, expand a Value cell to see the whole node/map/path, see a query plan as
a tree, and step back through earlier results. Completion offers the names the
database knows when Memgraph's schema feature is on, and the same keyword/
function vocabulary regardless. Everything the REPL does, the workbench does;
the wins are interaction, not new Core capability.

The workbench drives the async Core directly through a non-blocking event loop:
a query runs as a cancellable task that streams Records over a channel into the
results view, so the UI never freezes. It holds one Session and runs one query
at a time (ADR 0005's one-live-result guard); a second query while one runs is
refused with "session busy". See ADR 0010.

## User Stories

1. As an interactive user, I want a full-screen workbench by default, so that I
   get a richer console without opting in.
2. As a user on a terminal that cannot support the full-screen UI, I want the
   console to fall back to the line REPL automatically, so that it still works.
3. As a user who prefers the line REPL, I want `--plain` to select it, so that I
   keep the minimal mode.
4. As a script author, I want piping queries in to behave exactly as today, so
   that the new default Frontend does not affect automation.
5. As a user, I want `--color`/`NO_COLOR` to control colour within whichever
   Frontend runs, so that a monochrome terminal still gets the full-screen UI,
   just without colour.
6. As a user, I want a persistent multiline editor pane, so that I can compose
   complex queries comfortably.
7. As a user, I want to move the cursor anywhere in the query and edit it, so
   that I can fix a mistake without retyping.
8. As a user, I want Enter to run the query in the editor, so that submitting is
   one keystroke.
9. As a user, I want Shift+Enter or Ctrl+Enter to insert a newline on terminals
   that support it, so that multiline editing is natural.
10. As a user on a terminal that cannot distinguish those chords, I want a
    documented universal newline key (e.g. Alt+Enter / Ctrl+J) shown in the
    status hint, so that I can still add lines and Enter still submits.
11. As a user, I want to put several `;`-separated statements in the editor and
    run them all with one Enter, so that I can run a small script at once.
12. As a user, I want syntax highlighting as I type, so that I catch mistakes
    early (the same lexical signal the REPL gives).
13. As a user, I want keyword and function completion, so that I type faster.
14. As a user, I want completion to include the labels, relationship types, and
    property keys the database holds when Memgraph's schema feature is enabled,
    so that I complete against my real graph.
15. As a user on a database without the schema feature, I want completion to
    fall back silently to keywords/functions only, so that nothing breaks and no
    expensive scan runs.
16. As a user, I want to browse the database Schema in a sidebar, so that I can
    see what labels and properties exist.
17. As a user, I want to refresh the Schema on demand, so that I can pick up
    changes I have made.
18. As a user, I want results in a navigable table with a header that stays put
    while I scroll, so that I can read large answers.
19. As a user, I want rows to appear as they stream in, with a live count, so
    that I see progress on a big query.
20. As a user, I want to explore results far larger than the REPL's row cap, so
    that a big answer is navigable rather than refused.
21. As a user, I want to select a cell and expand a node/relationship/path/map
    Value into a detail view, so that I can read the whole Value.
22. As a user, I want to export the on-screen result to csv / jsonl / cypherl,
    so that I can take the data elsewhere (reusing the existing writers).
23. As a user running a long query, I want to cancel it with Ctrl-C and keep my
    session, so that a mistake does not cost me my shell.
24. As a user who cancels a streaming query, I want the rows that already
    arrived to stay on screen labelled "partial", so that I keep what I have
    seen.
25. As a user, I want a spinner / elapsed timer / "running…" state while a query
    runs, so that I know the workbench is working and not hung.
26. As a user, I want to keep scrolling, editing, and browsing while a query
    runs, so that the UI never freezes.
27. As a user who runs a second query while one is in flight, I want a clear
    "session busy — cancel first" message, so that I understand why it did not
    start.
28. As a user, I want `EXPLAIN`/`PROFILE` results rendered as a navigable
    operator tree (PROFILE annotated with hits/time), so that I can read a plan
    the way it is shaped.
29. As a user, I want plans to appear where results appear, so that the output of
    what I ran is where I look.
30. As a user, I want a history of past results I can step back and forward
    through, so that I can revisit an earlier query's answer.
31. As a user, I want my command history persisted across sessions, so that I can
    recall earlier queries (reusing the REPL's history).
32. As a user, I want to set and view query parameters (`:param` family) in the
    workbench, so that I separate data from query text as I do in the REPL.
33. As a user, I want notifications, stats, and timing surfaced in the status
    area, so that I learn about hints and performance.
34. As a user, I want a clear error message when a query fails without losing my
    session, so that I can fix and retry.
35. As a packager, I want the ratatui/crossterm dependencies behind a Cargo
    feature, so that a build that does not want the TUI stays lean.
36. As a contributor, I want the workbench logic testable without a terminal or a
    database, so that its behaviour is covered the way the REPL loop is.

## Implementation Decisions

Binding decision of record: **ADR 0010** (the workbench is the default
interactive Frontend, driving the async Core non-blocking over one Session; line
REPL behind `--plain`). This PRD implements it and must not contradict it, nor
ADR 0002 (Core/Frontend seam), ADR 0004 (streaming Records), ADR 0005
(one-live-result guard, RESET recovery), or ADR 0008 (Core tokens, Frontend
colour).

- **A new Frontend, not a new Core.** The workbench is a feature-gated module of
  the binary crate. The Core is untouched: per-cell Value text comes from the
  existing Value renderer, and the existing whole-result table renderer stays for
  the REPL/serial/import paths. The workbench lays cells out in a native ratatui
  table instead.

- **Workbench as a pure state + reducer.** The workbench is modelled as a pure
  state value and an `update(state, event) -> (state, effects)` function
  (Elm-style), exactly mirroring how the REPL's `run_loop` is split from its IO.
  Events are terminal input events and query-lifecycle events (record arrived,
  completed, errored, cancelled, schema loaded). Effects describe IO to perform
  (run a query, cancel, fetch schema, export). The ratatui draw and the real
  async execution are thin adapters at the edges, the analogue of the REPL's
  rustyline `LineSource` and `SessionRunner`.

- **Non-blocking execution.** A submitted query runs as a cancellable async task
  driven by the tokio runtime; Records stream back over a channel as
  query-lifecycle events into the reducer, which appends them to the current
  result. The render loop multiplexes terminal-input events and these channel
  events and never blocks. Ctrl-C issues a cancel that maps to Bolt `RESET`,
  recovering the Session per ADR 0005.

- **One Session, one query in flight.** The workbench holds a single Session.
  While a query is running, submitting another is rejected by the reducer with a
  "session busy — cancel first" status, honouring the one-live-result guard.
  Concurrent-query tabs (a Session per tab) are explicitly out of scope here.

- **Frontend selection + fallback.** A pure resolver maps `(--plain flag, stdin
  is a terminal, terminal supports the full-screen UI)` to the chosen Frontend:
  the workbench by default for a capable interactive terminal, the REPL when
  `--plain` is set or the terminal cannot support the UI. The piped/
  non-interactive path is selected as today and bypasses both. `--color`/
  `NO_COLOR` is resolved separately and governs colour within the chosen
  Frontend; a `--color=never` workbench is monochrome, not disabled.

- **Editor.** The editor pane is a text-input widget providing multiline editing,
  free cursor movement, selection, and undo. Enter submits the entire editor
  buffer; the existing query assembler splits it on `;` into one or more
  statements, run sequentially on the one Session. Submission is decoupled from
  the REPL's `;`-completeness rule — a buffer with no `;` runs as a single query.
  Shift+Enter / Ctrl+Enter insert a newline on terminals whose keyboard protocol
  distinguishes them (negotiated at startup); a universal newline key is the
  fallback everywhere and is shown in the status hint.

- **Highlighting is Frontend-neutral.** The token→category classification is
  lifted out of the REPL's ANSI emitter into a shared `HighlightCategory`
  (keyword, function, string, number, comment, parameter, plain) that the REPL
  maps to ANSI and the workbench maps to a ratatui style. This extends ADR 0008
  (the Frontend owns colour) to two colour-owning Frontends; the REPL's observable
  ANSI output is unchanged.

- **Results pane.** A native ratatui table with a pinned header and scrollable,
  lazily-rendered rows fed from the streaming channel; only the visible window is
  drawn. The REPL's row cap is relaxed to a high, configurable backstop (a memory
  guard, not a usability limit). A selected cell expands a Value into a detail
  view. The pane is **polymorphic**: an `EXPLAIN`/`PROFILE` query (detected by a
  pure predicate over the leading lexer token) renders the returned plan as a
  navigable, collapsible operator tree (PROFILE annotated with hits/time) instead
  of a table.

- **Result history.** Results are pushed onto a navigable history stack; the pane
  shows the latest with back/forward navigation across statements and submits. A
  multi-statement submit pushes one entry per statement.

- **Cancellation result.** On cancel, rows already streamed remain on screen and
  the result is labelled "partial" with its count; the Session is reset and ready
  for the next query.

- **Schema completion.** On connect (Session idle), the workbench fetches the
  Schema via Memgraph's schema feature when it is enabled and registers it as a
  second completion source alongside the static keyword/function vocabulary; a
  manual refresh re-fetches. When the feature is off or unavailable, the
  workbench degrades silently to static-only completion and never runs a graph
  scan to derive names. The Schema also backs the schema sidebar.

- **Layout.** Editor on top, results below, a one-line status bar; the
  editor/results split is resizable with a sensible default. The Schema sidebar,
  history, parameters, the plan tree, and the cell-detail view are toggleable
  drawers/overlays summoned by keybind, not always-on. The Schema drawer is
  absent when the server feature is off.

- **Reuse.** Command history (the REPL's persisted history), the parameter store
  and server-side `:param` evaluation, notification/stats/timing computation, and
  the csv/jsonl/cypherl writers are reused rather than reimplemented.

## Testing Decisions

A good test asserts **external behaviour** — what a user or a caller observes —
not internal structure, and survives a behaviour-preserving refactor. Seams,
highest first:

- **Workbench reducer (pure, primary seam).** Feed event sequences to
  `update(state, event)` with no terminal and no database and assert resulting
  state: pane focus, editor buffer after key events, statements produced by a
  multiline submit, the "session busy" rejection of a second query, partial-rows-
  labelled on cancel, plan-tree vs table mode selection, schema source populated
  vs empty, and result-history back/forward. This mirrors the REPL's fake-driven
  `run_loop` tests (scripted `LineSource` + `QueryRunner`) and carries the bulk of
  coverage.

- **Reused pure seams.** The query assembler's multi-statement split, the
  completer (including a registered schema source contributing candidates),
  `word_start`, and the new `HighlightCategory` classification are pure unit
  tests in the existing style. The REPL's existing ANSI highlight tests must stay
  green, proving the category refactor is behaviour-preserving.

- **Pure resolvers.** Frontend selection `(plain, is_tty, supports_tui) ->
  Frontend`, the `EXPLAIN`/`PROFILE` prefix predicate, and the `SHOW SCHEMA INFO`
  result → completion-name-set parser are pure functions tested without IO, in
  the `ColorChoice::resolve` style.

- **Schema fetch (integration, live Memgraph).** Against a real server with the
  schema feature enabled, assert the fetched names populate completion; with it
  disabled, assert the workbench degrades to static-only. Graduates the Session
  API integration pattern (testcontainers-backed `memgraph/memgraph`).

- **Cancellation (integration).** The RESET path and Session recovery are already
  covered by the Session error-taxonomy tests (ADR 0005); the workbench's handling
  of a cancel event is covered at the reducer seam.

- **Render smoke (thin).** A small number of ratatui `TestBackend` golden renders
  of representative states (table result, plan tree, partial-cancel label),
  asserting the drawn buffer. Kept thin because the reducer and rendering seams
  already carry the detail.

Live-Memgraph integration uses the existing testcontainers harness. Terminal
interaction itself is not driven by a real terminal in tests — the reducer seam
makes that unnecessary.

## Out of Scope

- **Concurrent-query tabs** (a Session per tab so two long queries run at once).
  ADR 0010 keeps one Session, one query in flight; this is a deliberate later
  increment the channel model already permits.
- **Graph visualisation / node-neighbour drill-in.** The bridge toward a
  graph-explorer product; not built here.
- **Mouse-driven interaction** beyond what the chosen widgets provide for free;
  the workbench is keyboard-first.
- **A configurable keybinding scheme.** A sensible fixed set ships first.
- **Output formats beyond the existing tabular / csv / jsonl / cypherl.**
- **Replacing or changing the Core**, the import modes, or the non-interactive
  piped path's behaviour.
- **In-TUI connection management** (switching endpoints, multi-server). The
  workbench connects once at startup as the REPL does.

## Further Notes

- ADR 0010 is the binding decision; ADRs 0002 / 0004 / 0005 / 0008 constrain the
  surrounding behaviour and must not be contradicted.
- CONTEXT.md's **Schema** term is authoritative for the completion/sidebar
  vocabulary: the labels, relationship types, and property keys the database
  holds, the open-vocabulary dual of the closed keyword/function set.
- Build-time verification items (not design forks): the exact Memgraph
  schema-introspection query and its enabling setting; crossterm keyboard-
  enhancement negotiation for distinguishing Shift/Ctrl+Enter; and the tree-widget
  choice for the plan view.
- Recommended build sequence even though all four headline features are v1:
  (1) workbench shell + editor + streaming results table + cancellation, then
  (2) cell-expand + export, then (3) live schema completion + sidebar, then
  (4) EXPLAIN/PROFILE plan tree — shipping after each for earlier feedback.
