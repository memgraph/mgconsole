# PRD: Workbench ergonomics — refinement pass

Status: ready-for-agent

## Problem Statement

The console-ergonomics batch gave the Workbench tabs, in-result search, mouse
support, themes, auto-format, and plan tables — but a design review (grilling
session) surfaced UX rough edges and a few model inconsistencies that a finished
client shouldn't ship with:

- `Ctrl+W` closes a tab — the universal *delete-word* chord — destroying a Buffer
  mid-edit, irrecoverably (editor undo is also discarded on buffer replacement).
- `:set display vertical` is silently ignored in the Workbench (it has no live
  vertical mode), so the Setting lies.
- There's no way to copy a Value out: mouse capture disables native selection and
  no copy action exists.
- A failed query vanishes as a transient status — it can't be correlated back to
  the query that caused it once you've moved on.
- Tabs are anonymous numbers that overflow the bar; result columns are equal-width
  and truncate wide Values; the theme can't recolour chrome.

This pass closes those, grounded in the sharpened `CONTEXT.md` vocabulary and
ADR 0015.

## Solution

A batch of thin vertical slices, each demoable on its own, hung off the decisions
settled in the grilling session and recorded in `CONTEXT.md` / ADR 0015. No new
architecture — these refine existing Workbench surfaces (the results table, the
Buffer/tab model, the Result history, the keybinding/command vocabulary, the
theme, and the overlays).

## Key design rulings (from grilling)

- **Display mode is buffered-render-only** (`CONTEXT.md` "Display mode"): the
  Workbench's answer to a row that won't fit is content-aware columns + cell-
  expand, not a vertical dump. The `settings.rs` "means the same in the Workbench"
  claim was corrected. *(Resolved doc-only during grilling; no issue.)*
- **A live query belongs to its origin Buffer** (`CONTEXT.md` "Buffer"): its
  Records stream there even while another Buffer is shown, so switching-to-view is
  always allowed — only a *second submit* is refused (ADR 0005, unchanged).
- **The editor owns single-`Ctrl`/`Alt`+letter chords**; Workbench gestures move
  off them. Buffer *navigation* is a gesture (`Ctrl+PageUp/PageDown`); Buffer
  *close* is a **Workbench command** `:close` (`CONTEXT.md` "Workbench command") —
  destructive, so deliberate-by-typing.
- **Copy is OSC 52, not a native clipboard library** (ADR 0015): pure bytes, no
  FFI (ADR 0001), works over SSH.
- **The Result history is the correlation surface** (`CONTEXT.md` "Result
  history"): every submission is a navigable entry pairing the query with its
  outcome; a failed query is an entry carrying its query + error.
- **Tabs auto-title** from a query snippet and the bar windows; the **theme
  extends** to chrome (border/selection/status) with a `light` built-in.

## Non-goals

- A Workbench vertical record view — display is buffered-only (resolved above).
- User-nameable tabs — Buffers stay ephemeral and auto-titled.
- A separate session message log — the Result history is the correlation surface.
- Copy as a re-serialization (jsonl/cypherl) — yank copies rendered text.
- A native clipboard dependency — OSC 52 only (ADR 0015).
