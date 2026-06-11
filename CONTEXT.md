# Context

Glossary for the Rust Memgraph console. Terms only — no implementation detail.
When a term below conflicts with how a word is used in code or conversation,
the definition here wins until deliberately changed.

## Core / Frontend

- **Core** — the part of the tool that owns a database connection, turns a
  query plus parameters into a stream of results, and renders database values
  to text. It knows nothing about how a human drives it.
- **Frontend** — a way of driving the Core. The interactive REPL is the first
  Frontend; a full-screen terminal UI and a non-interactive script runner are
  other Frontends over the same Core.
- **Meta-command** — an instruction the user types to drive the console itself
  rather than the database, prefixed with `:` and recognised before the line is
  treated as Cypher (e.g. `:help`, `:param`, `:quit`). The console parses every
  meta-command into one shared vocabulary, so a given command behaves
  consistently across interactive Frontends; each Frontend may then present its
  effect in its own way. The cohesion spine for features that drive the console.
  (A few `:`-commands act on a Workbench-only concept and so are Workbench
  commands instead, below.)
- **Workbench gesture** — a TUI-only interaction (a key chord or mouse action)
  with no meaning in the line REPL, e.g. switching Buffer tabs or searching
  within a displayed result. The chord/mouse counterpart to a Meta-command. A
  Workbench concept may carry both: navigation by gesture, lifecycle by command.
- **Workbench command** — a typed `:`-command that drives a Workbench-only
  concept and so has no meaning in the line REPL (which reports it unknown),
  e.g. `:close` on a Buffer. The typed counterpart to a Workbench gesture, used
  where a deliberate, named invocation is wanted over a chord — notably for a
  destructive action. _Contrast_: Meta-command (cross-frontend).
- **Buffer** — one tab in the Workbench: an independent line of inquiry owning
  its own editor text and its own result and result history. All Buffers share
  the single Session, so they share the connection, profile, Read-only mode,
  open Transaction, `:param` store, Schema, and Settings — and only one query is
  ever live across all of them at once (a second _submit_ is refused, not
  queued; ADR 0005). A live query belongs to the Buffer it was submitted from:
  its Records stream into that Buffer even while another is shown, so switching
  Buffers to read another line of inquiry is always allowed — only a second
  submit is refused. Buffers are ephemeral and not persisted across runs.
- **Command line** — the Workbench's modal input surface for typed
  `:`-commands (Meta-commands and Workbench commands): a one-line prompt that
  opens on `:` while the Buffer editor is empty, or on demand anywhere, runs on
  Enter, and dismisses on Esc, returning focus to where it was. It is how the
  Workbench presents the typed `:`-vocabulary, keeping the Buffer editor for
  Cypher alone — the Workbench counterpart to the REPL prompt, where the same
  vocabulary is typed inline. Because the editor holds only Cypher, its `Tab`
  means completion alone (ADR 0017). _Contrast_: the Buffer editor (Cypher only).

## Query text

- **Token** — a lexical unit of Cypher _query text_, recognised by its shape
  alone without consulting the database: a keyword, function name, string,
  number, query parameter, comment, or punctuation. The input-side counterpart
  to a Value (which is database output). One token reading underlies the three
  things the console does with raw query text: splitting input into queries,
  detecting a query's clauses, and colouring input as it is typed.

## Session and results

- **Session** — a single live conversation with one Memgraph server over which
  queries run and results return, in order.
- **Endpoint** — the host and port identifying the one Memgraph server a Session
  connects to: the _where_ of a connection, distinct from the _how_
  (authentication and transport security).
- **Database** — one of possibly several named graph stores a single Memgraph
  server (one Endpoint) hosts under multi-tenancy. A Session has exactly one
  _active_ Database at a time; `:use` switches it, while `:connect` swaps the
  whole Session to a different Endpoint. The Schema is metadata of the active
  Database, so switching Databases refetches it.
- **Transaction** — the unit of work a Session commits or rolls back as a whole.
  A Session is either in _autocommit_ — each query its own transaction, the
  default — or in an _open transaction_ the user has bracketed with `:begin`
  until `:commit` or `:rollback`. The distinction governs reconnect: the console
  silently re-establishes a dropped connection in autocommit, but never silently
  resurrects an open transaction (ADR 0011).
- **Transaction episode** — one lifetime of an open Transaction, from `:begin`
  to the `:commit` or `:rollback` that ends it. Session-scoped — all Buffers
  share the one open Transaction — so its statements may be submitted from
  several Buffers; each is numbered by the episode and by its ordinal within it,
  and carries the episode's final _disposition_ (open, committed, or rolled
  back). The unit a Result history entry is tagged against, so a query run inside
  a transaction is reviewable as what it was — the Nth statement of an episode
  that was ultimately committed or undone — rather than a bare success whose rows
  may since have been rolled away.
- **Read-only mode** — a session-wide safety guard that sets every Transaction's
  Bolt access mode to READ, so the server itself rejects writes. Memgraph
  enforces it, not the console — the Clause scanner is never asked to police it.
  It can be turned _on_ at runtime but only turned _off_ deliberately, at connect
  time or via a connection profile, so a guard pointed at production can't be
  flipped away by accident.
- **Value** — a single piece of data Memgraph returns: a node, relationship,
  path, list, map, scalar, temporal value, spatial point, or enum. The console's
  defining job is to render every Value faithfully.
- **Record** — one row of a result: an ordered set of Values, one per result
  column.
- **Record stream** — the Records of one query, consumed one at a time rather
  than held all at once. _Avoid_: result stream, records stream.
- **Query result** — the whole answer to one query: its header (column names),
  its Record stream, and the trailing summary (timing, notifications, stats) that
  the server sends only after the last Record. Always paired with the query that
  produced it, so the two can be reviewed together. _Avoid_: result set, response.
- **Result history** — the per-Buffer stack of past submissions, one entry each,
  navigated in place. Every entry pairs the originating query with its outcome: a
  Query result with its Notifications, or — when the query failed — the error in
  place of Records. Navigating the stack always shows an outcome together with the
  query and Notifications that belong to it, so a failure is reviewable, not a
  status message that has scrolled away. _Avoid_: result log, output history.
- **Query parameter** — a named value supplied alongside a query and referenced
  inside it, so the query text and its data stay separate.
- **Notification** — advisory information Memgraph attaches to a result (e.g. a
  performance hint), distinct from the result data itself.
- **Enum** — a Memgraph Value naming one member of a user-declared enumerated
  type, written `Type::Member` (e.g. `Status::Active`). The console treats it as
  a first-class Value, not as the map Memgraph happens to transmit it as.
- **Display mode** — how a _buffered_ Query result is laid out as text:
  `tabular` (a box table), `vertical` (one `column: value` line per field, for
  rows too wide to tabulate), or `auto` (tabular until a row won't fit, then
  vertical). A Setting (`:set display`) that governs the REPL and the
  non-interactive renders only — the Workbench shows results in a live,
  navigable table and expands a wide Value on demand, so it never consults the
  display mode. _Avoid_: output format (the serialization below), layout.
- **Output format** — the serialization a Query result is written in for
  non-interactive output, redirection, and export: `csv`, `jsonl`, `cypherl`, or
  `table`. One shared vocabulary behind the batch `--output-format`, the `:o`
  redirect, and the Workbench export. Distinct from Display mode: `table` here is
  a file rendering, not the interactive layout choice. _Avoid_: display mode.

## Schema

- **Schema** — the labels, relationship types, and property keys present in the
  active Database. It is metadata _about_ the graph, distinct from a Query
  result's data: the console fetches it so it can complete and browse the names
  the database knows. The open-vocabulary dual of the closed keyword/function
  vocabulary a Token is classified against — the console knows keywords without
  the database, but can only know labels and property keys by asking it.

## Console configuration

- **Setting** — a value that controls how the console itself behaves (result
  display mode, read-only mode, theme, …), as opposed to query data. Read and
  changed through one `:set` command, kept rigorously separate from the
  `:param` store. Resolved by precedence: built-in default < config file < CLI
  flag < runtime `:set`. _Contrast_: Query parameter (data bound into a query).
- **Connection profile** — a named bundle of the _where_ and _how_ of a
  connection (Endpoint, authentication, transport security, Read-only mode, and
  Setting overrides), stored in the hand-edited config file so a connection can
  be named once and reused. The active profile is shown in the REPL prompt and
  the Workbench status bar.
- **Named query** — a query saved under a name for later recall, kept in a
  separate tool-managed file the console rewrites on save and delete (distinct
  from the hand-edited config file, so rewriting never clobbers the user's
  comments).

## Import / export

- **Import mode** — the discipline by which a batch of queries from a file is
  run:
  - **Serial** — run queries one after another in the given order.
  - **Batched-parallel** — run queries concurrently in batches for speed,
    accepting that correctness now depends on input ordering and on retrying
    conflicts.
  - **Parser** — inspect the queries and report on them without running any.
- **cypherl** — a plain-text file of Cypher queries, one logical query at a
  time, used to move a database's contents out of and back into Memgraph.
- **Batch** — a group of queries from an Import that are submitted together as
  one unit of concurrency and retry.
- **Vertices-first ordering** — the requirement that node-creating queries
  appear before the edge-creating queries that depend on them, so a
  Batched-parallel import produces a correct graph.
