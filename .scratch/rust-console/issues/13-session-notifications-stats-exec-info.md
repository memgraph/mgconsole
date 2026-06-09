# 13 — Notifications + stats + verbose execution info

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Surface the metadata Memgraph returns with a result through the Session: query
Notifications, execution stats, and the optional verbose execution info (cost,
parse, plan, execute times). This metadata arrives in the **trailing Bolt
`SUCCESS`** that follows the last Record, so it fills the `QueryResult.summary`
slot established in slice 02 (ADR 0004) — readable once the Record stream is
drained. Display is wired into the REPL in slice 16. Integration tested against a
live container.

## Acceptance criteria

- [ ] `QueryResult.summary` exposes any Notifications attached to a result
- [ ] `summary` exposes execution stats when present
- [ ] Verbose execution info (cost/parse/plan/execute) is exposed in `summary` when requested
- [ ] Absence of any of these is represented cleanly (no error); `summary` is readable after the records drain
- [ ] Integration tests assert each against a container with queries that produce them

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
