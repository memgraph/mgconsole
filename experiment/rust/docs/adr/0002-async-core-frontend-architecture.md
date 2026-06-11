# 2. Async core with a core-library / frontend seam

Date: 2026-06-10

## Status

Accepted

## Context

The Rust console is intended to grow beyond a port of `mgconsole`. The roadmap
that is real enough to design for now includes: a full-screen terminal UI
frontend, in-flight query cancellation (Ctrl-C sends a Bolt `RESET` rather than
killing the process), streamed/paginated large result sets, and non-interactive
scripting (run a file or a single query, machine-readable output, CI exit
codes). Only "stay a lean REPL" was explicitly *not* chosen.

Two forces follow from that roadmap:

- The chosen Bolt stack is async (tokio) — see ADR 0001. A REPL, by contrast,
  is an inherently blocking read-eval-print loop. The two must be reconciled.
- Most high-value roadmap features (cancel, stream, live schema-aware
  autocomplete, watch/dashboard, cluster routing) are painful in a blocking
  design and natural in async. The features that are indifferent to the runtime
  (scripting, JSON output) do not fight async. So async is a deposit to be
  spent, not a tax.

A separate, larger force is that a terminal UI is not a REPL — it is an event
loop — and scripting wants the engine usable with no interactive layer at all.
Both only stay cheap if the engine is decoupled from any one way of driving it.
Today's `mgconsole` tangles these: rendering reaches into the line editor, and
each import mode re-opens its own connection.

Output format interacts with streaming: aligned tabular output needs every row
before it can size columns, which forbids streaming; row-oriented formats do
not.

## Decision

- **Runtime:** one multi-threaded **tokio** runtime. The interactive REPL and
  serial import stay synchronous code that calls `block_on` at the Bolt
  boundary per operation. Individual features are promoted to `select!` / tasks
  as they are built (cancellation, parallel import), because the runtime is
  already present.
- **Seam:** a **core library** owns the Session, the Value model, value
  rendering, and the import engine, with no dependency on any frontend. The
  interactive REPL, the future terminal UI, and the script runner are
  **frontends** over that core.
- **Records are streaming-first.** The core yields Records as a stream.
  Rendering is **format-dependent**: `csv` / `jsonl` / `cypherl` stream
  row-by-row with bounded memory; `tabular` buffers (it serves a human reading
  human-scale output) up to a row cap, past which it warns and points to a
  streaming format.

## Consequences

- A small amount of async "tax" on the REPL today (a `block_on` per query) buys
  cheap access to every roadmap feature later, with no re-architecture.
- The terminal UI and script runner become additive frontends rather than
  rewrites; the core is independently testable and usable headless.
- The tabular path retains a buffer-all renderer (e.g. `comfy-table`) while the
  streaming formats use row-oriented writers — a deliberate asymmetry, not an
  inconsistency.
- New contributors must respect the seam: frontend concerns (line editing,
  terminal drawing, prompts) never leak into the core.
