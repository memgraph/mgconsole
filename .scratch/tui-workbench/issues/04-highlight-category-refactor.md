# 04 — Frontend-neutral `HighlightCategory` refactor

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Lift the token→colour-category classification out of the REPL's ANSI emitter
into a shared, Frontend-neutral `HighlightCategory` (keyword, function, string,
number, comment, parameter, plain). The REPL maps the category to ANSI exactly as
today; the category mapping is exposed so the workbench (slice 05) can map it to
a ratatui style instead. This extends ADR 0008 (the Frontend owns colour) to two
colour-owning Frontends: one token classification, two renderings.

A pure refactor behind the existing highlighter tests — the REPL's observable
ANSI output does not change.

## Acceptance criteria

- [ ] A shared `HighlightCategory` (the seven categories) is produced from the
      Core lexer's tokens + the keyword/function tables, independent of any ANSI
      or terminal styling.
- [ ] The REPL highlighter maps `HighlightCategory` to its existing ANSI colours;
      all existing REPL highlight tests pass unchanged (behaviour-preserving).
- [ ] The classification is covered by pure unit tests independent of ANSI.

## Blocked by

- None - can start immediately
