# PRD: Console ergonomics — power-user parity

Status: ready-for-agent

## Problem Statement

The console renders Memgraph values faithfully and streams, scripts, and imports
well, but it lags the everyday CLIs its users come from — `cypher-shell`, `psql`,
`pgcli` — on the small ergonomics that make a terminal client feel finished:
explicit transactions, switching database or server without restarting, a way to
run a file or re-run a query on a timer, saved queries, connection profiles, a
safety guard against writing to prod, and a few Workbench affordances (vertical
display, tabs, search, mouse). Today each of these would be a one-off, and a pile
of one-offs drifts into an inconsistent surface.

## Solution

A cohesive batch of features hung off one spine — the **Meta-command** (`CONTEXT.md`):
every console-driving instruction is a `:`-prefixed command parsed once into a
single vocabulary and honoured by both interactive Frontends, so a command means
the same thing in the REPL and the Workbench. Console behaviour is configured
through one **`:set`** mechanism (kept rigorously separate from the `:param`
query-data store), resolved by precedence (default < config file < CLI flag <
runtime `:set`) and persisted in a hand-edited `~/.mgconsole/config.toml`, with
named queries in a tool-managed `~/.mgconsole/queries.toml`. Session semantics
(transactions, `:use`/`:connect`, read-only) live in the Core so every Frontend
gets them, including scripts. Workbench-only affordances are openly **Workbench
gestures**, not pretend cross-Frontend commands.

See `CONTEXT.md` for the vocabulary (Meta-command, Setting, Connection profile,
Named query, Database, Transaction, Read-only mode, Buffer) and:

- **ADR 0011** — explicit transactions add a Session state; reconnect becomes
  state-dependent (never silently resurrects bracketed work).
- **ADR 0012** — console state lives in `~/.mgconsole`, a clean break from
  mgconsole's `~/.memgraph`.

## Key design rulings (from grilling)

- The typed `:`-command vocabulary is the unit of cohesion; Workbench gestures
  (tabs, mouse, search, theme, auto-format) are openly TUI-only.
- Vertical/expanded display is a **Core** render mode beside tabular, default
  `auto` (flip to vertical when a row won't fit), composing with fit-to-screen.
- One **`:set`** for every console setting; never merged with `:param`.
- Read-only is **server-enforced** (Bolt READ access mode), session-wide,
  profile-storable; can be turned on at runtime but off only at connect/profile.
- `:connect` swaps the whole Session (one server per Session holds); `:use`
  switches the active Database within a Session (multi-tenancy in scope).
- `:o` is a **one-shot** redirection reusing the existing format writers and a
  shared `csv|jsonl|cypherl|table` vocabulary (also used by batch `--output` and
  Workbench export).
- Named queries are reusable text **templates** with live `$param` binding;
  recall loads to the input, never auto-runs. Flat verbs: `:save`/`:saved`/
  `:load`/`:forget`.
- `:source` stops on first error and echoes; `:watch` defaults to the last query
  at 2s, stops on Ctrl-C, redraws in place, and is refused inside a transaction.
- Workbench tabs (**Buffers**) are organizational over one shared Session — no
  cross-tab parallelism; per-tab editor+results, session-global everything else.

## Non-goals

- Cross-tab / cross-Session parallel execution (the import worker pool owns
  concurrency, ADR 0006).
- Migration of state from `~/.memgraph` (pre-release; clean switch, ADR 0012).
- Client-side write detection (read-only is server-enforced, not policed by the
  Clause scanner).
