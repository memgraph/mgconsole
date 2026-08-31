# 10. The interactive Frontend is a full-screen TUI workbench driving the async Core non-blocking over one Session

Date: 2026-06-10

## Status

Accepted

Amended by ADR 0019: the startup selection logic below still decides which
Frontend you *start* in, but the choice is no longer permanent — the two
interactive Frontends can be switched at runtime over a preserved Session.

## Context

The first interactive Frontend (ADR 0002) is a line-based REPL on rustyline: it
reads a line, then `block_on`s the query to completion, draining every Record
before the prompt returns. That is correct for a line REPL — there is nothing to
repaint while a query runs — but it forecloses the features people actually ask a
graph console for, which the PRD parks as roadmap: cancel an in-flight query and
keep the session (story 44), stream huge results instead of buffering them, and
a full-screen terminal UI (story 45). A blocking model cannot deliver any of
them: the screen is frozen for the query's duration.

The seam for a second Frontend already exists. The Core is async behind a
`block_on` boundary, knows nothing about how a human drives it (ADR 0002), and
yields Records as a stream (ADR 0004). The input-side logic a full-screen UI
needs is already pure and Frontend-neutral — the Core lexer (ADR 0008),
`syntax::highlight()`, the extensible `Completer`, and the `QueryAssembler`.
ADR 0002 explicitly anticipated this path: "Cancellation and parallel import
promote to `select!` / tasks." ADR 0005's one-live-result guard and its
RESET-based recovery are the cancellation primitive. So a terminal UI is a new
Frontend over an unchanged Core, not a rewrite.

rustyline and ratatui cannot share the terminal — ratatui takes the alternate
screen and raw mode and runs its own input event loop — so the two interactive
Frontends are a mutually exclusive choice made at startup, orthogonal to the
non-interactive piped path (already selected by `stdin().is_terminal()`).

## Decision

- **The default interactive Frontend is a full-screen TUI workbench.** The line
  REPL is retained behind `--plain`. The non-interactive piped path is unchanged
  and bypasses both. When the terminal cannot support the TUI (e.g. `TERM=dumb`,
  not a capable tty), the binary **auto-falls back to the REPL** rather than
  erroring; `--plain` is the manual escape hatch. `--color`/`NO_COLOR` governs
  colour *within* whichever Frontend runs (a `--color=never` TUI is monochrome,
  not disabled) — it does not pick the Frontend.

- **The TUI drives the async Core directly via a non-blocking event loop.** A
  query runs as a cancellable task that streams Records back over a channel into
  the results view; the render loop never blocks, so the UI stays responsive
  (scroll, edit, browse) and shows live progress while a query runs. Ctrl-C
  cancels the in-flight query via Bolt `RESET`, keeping the session (ADR 0005).

- **One shared Session, one query in flight.** Panes and tabs are *views* over a
  single connection; starting a second query while one runs is refused
  ("session busy — cancel first"), honouring ADR 0005's one-live-result guard
  with no new machinery. Concurrent-query tabs (a Session per tab, N connections
  as in ADR 0006) are a deliberate later increment; the channel model already
  generalises from one task to N.

- **Shipped as a feature-gated module.** ratatui and crossterm are compiled in
  only under a Cargo feature, so the default binary stays lean while the TUI
  stabilises (ADR 0002 permits future crates/modules).

## Consequences

- Two roadmap items become working features almost for free: in-flight
  cancellation (the loop + ADR 0005 reset-recovery already exist) and
  streaming results display (Records already stream; the REPL's row cap relaxes
  to a high, configurable backstop because only the visible window is drawn).
- The Core is untouched. The TUI reuses the Core's per-Value rendering
  (`render::tabular`) for each table cell but lays cells out in a native ratatui
  `Table`; `render_table` stays for the REPL/serial/import paths. The principle:
  the Core renders each Value to text; each Frontend lays the cells out its own
  way.
- There are now two colour-owning Frontends, so token classification is lifted
  to a Frontend-neutral `HighlightCategory` that the REPL maps to ANSI and the
  TUI maps to a ratatui `Style` — extending, not contradicting, ADR 0008 ("the
  Frontend owns colour").
- Making a young full-screen UI the default (over keeping the proven REPL
  default) is a deliberate bet that the workbench is the better first experience;
  the auto-fallback and `--plain` bound the downside. Recording it here stops a
  future reader from "restoring" the REPL as the default and silently undoing the
  bet.
- Tabs are organisational, not parallel, in this decision; a user who needs two
  long queries at once is the signal to promote to Session-per-tab.
