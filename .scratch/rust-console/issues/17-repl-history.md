# 17 — Persistent REPL history

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Command history for the REPL Frontend: persist entered queries across sessions
to a history file, with the storage location configurable and an option to
disable history entirely. Honour the same precedence as today (explicit flag,
environment override, default directory). Reuse reedline's history support.

## Acceptance criteria

- [ ] Queries are saved to a history file and recalled in a later session
- [ ] The history location is configurable via flag
- [ ] An environment override takes precedence over the default location
- [ ] no-history disables reading and writing history
- [ ] A missing/!creatable history directory is handled with a clear message

## Blocked by

- `.scratch/rust-console/issues/16-repl-execute-loop.md`
