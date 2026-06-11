# PRD: Workbench ergonomics — round 2 (command line, transactions, clipboard)

Status: ready-for-agent

## Problem Statement

A live-driving session of the Workbench surfaced five rough edges, grilled into
shared decisions and recorded in `CONTEXT.md` (new terms **Command line**,
**Transaction episode**) and ADRs 0017 / 0018:

- **`Tab` is overloaded** — it both triggers completion and, with nothing to
  complete, cycles pane focus. The dual meaning is confusing.
- **No command surface** — the Buffer editor is one dual-purpose surface that
  accepts both Cypher and typed `:`-commands; there is no dedicated command line.
- **History under-correlates transactions** — a successful entry shows its query,
  but a query run inside a `:begin` transaction gives no sign of its position or
  whether the transaction was ultimately committed or rolled away.
- **The "in a transaction" state is easy to miss** — the `[tx]` marker is too dim.
- **Yank doesn't reach the system clipboard** — OSC 52 silently no-ops on a local
  terminal that doesn't honour it, with no fallback and no helper.

## Solution

A batch of thin vertical slices, each demoable on its own, grounded in the
sharpened `CONTEXT.md` vocabulary and ADRs 0017 (modal command line; editor is
Cypher-only; `Tab`/`Shift+Tab`) and 0018 (clipboard helper process over OSC 52).
No new architecture — these refine existing Workbench surfaces (editor key
dispatch, the status bar, the Result history, the theme, and the IO edge).

## Key design rulings (from grilling)

- **Modal `:` command line; the editor holds only Cypher** (ADR 0017). `:` on an
  empty buffer or `Ctrl+G` anywhere opens it; Enter runs, Esc cancels, focus
  returns. Editor submit no longer routes `:`-lines to the meta parser.
- **`Tab` is completion-only; `Shift+Tab` is prev-candidate / focus-switch**
  (ADR 0017). `Ctrl+;` was rejected for the opener (unreliable) in favour of
  `Ctrl+G`, for the same reason ADR 0016 rejected `Ctrl+Shift+Z`.
- **Transaction episode** (`CONTEXT.md`): a Session-scoped `:begin`→end lifetime,
  numbered, with per-statement ordinals and a final disposition; the Result
  history tags entries against it and resolves disposition retroactively across
  all Buffers.
- **Clipboard helper process, OSC 52 fallback** (ADR 0018): a subprocess is no
  FFI, so ADR 0001 stays intact; OSC 52 remains the SSH/no-helper path.

## Non-goals

- A native clipboard FFI crate (`arboard`) — forbidden by ADR 0001; helper
  subprocess only (ADR 0018).
- Running interleaved `:`/Cypher scripts by submitting them in the editor — that
  is `:source`'s job (ADR 0017).
- A persistent always-on command pane — the command line is modal (ADR 0017).
- User-nameable transactions — episodes are auto-numbered per Session.
