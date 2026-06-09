# 13 — Notifications + stats + verbose execution info

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Surface the metadata Memgraph returns with a result through the Session: query
Notifications, execution stats, and the optional verbose execution info (cost,
parse, plan, execute times). The Session exposes these alongside the Records;
display is wired into the REPL in slice 16. Integration tested against a live
container.

## Acceptance criteria

- [ ] The Session exposes any Notifications attached to a result
- [ ] The Session exposes execution stats when present
- [ ] Verbose execution info (cost/parse/plan/execute) is exposed when requested
- [ ] Absence of any of these is represented cleanly (no error)
- [ ] Integration tests assert each against a container with queries that produce them

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
