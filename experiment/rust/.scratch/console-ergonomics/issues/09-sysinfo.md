# 09 — `:sysinfo` server status

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add a `:sysinfo` Meta-command that interrogates the connected Memgraph server and
renders its status — version, storage info, and transaction/runtime details the
server exposes — through the normal value-rendering path. Thin: it runs the
server's status query (or queries) and renders the resulting Records like any
other result. Honoured identically in the REPL and Workbench.

## Acceptance criteria

- [x] `:sysinfo` returns and renders the server's version and storage/runtime
      information, verified against a live database (`SHOW VERSION`,
      `SHOW STORAGE INFO`).
- [x] Output renders through the existing value renderer (respecting the current
      `display` mode), not a bespoke formatter — `:sysinfo` runs the queries
      through the same execute/render path as a typed query.
- [x] `:sysinfo` behaves identically in the REPL and Workbench.
- [x] A query that a server does not support is reported as an error and the rest
      still run (each status query is independent).

## Blocked by

None - can start immediately.

## Comments

Implemented (AFK). Deliberately thin (no Core change).

- A shared `repl::SYSINFO_QUERIES = ["SHOW VERSION", "SHOW STORAGE INFO"]`
  (probed to work on Community).
- REPL: extracted `execute_query` (the per-query render+summary+overflow body the
  loop already had); `:sysinfo` runs each status query through it, honouring the
  current `display` mode. A failing query reports and the next still runs.
- Workbench: `:sysinfo` submits the status queries as a normal batch (first runs,
  rest queue), so each lands in the result pane rendered like any other result.
- Live integration test confirms `SHOW VERSION` returns a string and
  `SHOW STORAGE INFO` returns name/value rows the renderer handles as-is.
