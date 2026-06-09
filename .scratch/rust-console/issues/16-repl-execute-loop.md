# 16 — REPL execute loop (reedline)

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The interactive REPL Frontend's core loop, built on reedline: read input
(assembled into queries via slice 15), run each through the Session, render the
result (tabular via slice 08), and print the round-trip timing and row-count
summary. Surface query errors without ending the session and surface
fatal-error reconnect attempts (slice 14). Exit on Ctrl-D and `:quit`.

This is the first demoable interactive shell.

## Acceptance criteria

- [ ] reedline reads multiline queries with a continuation prompt
- [ ] Each completed query runs and its result renders as tabular
- [ ] Per-query round-trip time and a row-count summary print after results
- [ ] A query error is shown and the prompt returns (session survives)
- [ ] A fatal connection error shows reconnect attempts and resumes on success
- [ ] Ctrl-D and `:quit` exit cleanly

## Blocked by

- `.scratch/rust-console/issues/08-tabular-layout.md`
- `.scratch/rust-console/issues/14-session-error-taxonomy-reconnect.md`
- `.scratch/rust-console/issues/15-line-query-parsing.md`
