# 16 — REPL execute loop (rustyline)

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The interactive REPL Frontend's core loop, built on **rustyline** (chosen over
reedline: every requirement maps to a first-class rustyline `Helper` trait —
`Validator` for multiline, `Highlighter`/`Completer` for slice 18,
`FileBackedHistory` for slice 17 — and the line editor is frontend-local behind
the ADR-0002 seam, so the choice carries no Core lock-in). Read input (assembled
into queries via slice 15), run each through the Session, render the result
(tabular via slice 08), and print the round-trip timing and row-count summary.
Surface query errors without ending the session and surface fatal-error
reconnect attempts (slice 14). Exit on Ctrl-D and `:quit`.

This is the first demoable interactive shell.

## Acceptance criteria

- [x] rustyline's `Validator` keeps editing until the `QueryAssembler` (slice 15) reports a complete query, with a continuation prompt
- [x] Each completed query runs and its result renders as tabular
- [x] Per-query round-trip time and a row-count summary print after results
- [x] A query error is shown and the prompt returns (session survives)
- [x] A fatal connection error shows reconnect attempts and resumes on success
- [x] Ctrl-D and `:quit` exit cleanly

## Blocked by

- `.scratch/rust-console/issues/08-tabular-layout.md`
- `.scratch/rust-console/issues/14-session-error-taxonomy-reconnect.md`
- `.scratch/rust-console/issues/15-line-query-parsing.md`
