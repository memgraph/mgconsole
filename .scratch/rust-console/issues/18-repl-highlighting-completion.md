# 18 — Syntax highlighting + static keyword/function completion

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

REPL Frontend ergonomics via reedline's highlighter and completer: syntax
highlighting of Cypher as the user types, and autocompletion from the built-in
tables of Memgraph keywords, Cypher keywords, and functions (the static tables
carried over from `mgconsole`). The completer architecture should leave room for
live schema-aware completion later (out of scope here).

## Acceptance criteria

- [ ] Cypher input is syntax-highlighted as typed
- [ ] Completion offers Memgraph keywords, Cypher keywords, and functions from the static tables
- [ ] Highlighting/completion can be toggled consistently with today's term-colors behaviour
- [ ] The completer is structured so a live-schema source can be added later without rework

## Blocked by

- `.scratch/rust-console/issues/16-repl-execute-loop.md`
