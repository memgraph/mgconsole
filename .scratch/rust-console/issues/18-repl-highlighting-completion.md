# 18 — Syntax highlighting + static keyword/function completion

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

REPL Frontend ergonomics via rustyline's `Highlighter` and `Completer` (see
slice 16 for the rustyline-over-reedline choice): syntax highlighting of Cypher
as the user types, and autocompletion from the built-in tables of Memgraph
keywords, Cypher keywords, and functions (the static tables carried over from
`mgconsole`). The completer architecture should leave room for live schema-aware
completion later (out of scope here).

## Acceptance criteria

- [x] Cypher input is syntax-highlighted as typed
- [x] Completion offers Memgraph keywords, Cypher keywords, and functions from the static tables
- [x] Highlighting/completion can be toggled consistently with today's term-colors behaviour
- [x] The completer is structured so a live-schema source can be added later without rework

## Blocked by

- `.scratch/rust-console/issues/16-repl-execute-loop.md`
