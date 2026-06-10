# 09 — `:sysinfo` server status

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add a `:sysinfo` Meta-command that interrogates the connected Memgraph server and
renders its status — version, storage info, and transaction/runtime details the
server exposes — through the normal value-rendering path. Thin: it runs the
server's status query (or queries) and renders the resulting Records like any
other result. Honoured identically in the REPL and Workbench.

## Acceptance criteria

- [ ] `:sysinfo` returns and renders the server's version and storage/runtime
      information, verified against a live database.
- [ ] Output renders through the existing value renderer (respecting the current
      `display` mode), not a bespoke formatter.
- [ ] `:sysinfo` behaves identically in the REPL and Workbench.
- [ ] A server that does not expose a given field degrades gracefully (no crash,
      clear partial output).

## Blocked by

None - can start immediately.
