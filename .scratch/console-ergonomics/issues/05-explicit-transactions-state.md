# 05 — Explicit transactions: the Session transaction state

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Give the Session an explicit Transaction state (`CONTEXT.md`, ADR 0011). It is
either in *autocommit* (today's behaviour, each query its own transaction) or in
an *open transaction* the user brackets with `:begin` until `:commit` or
`:rollback`. Queries between run within the open transaction. `:begin`/`:commit`/
`:rollback` are Meta-commands honoured identically in the REPL and the Workbench.
The one-live-result guard (ADR 0005) composes unchanged — still one live stream
at a time, now inside the transaction. A query error inside a transaction poisons
it; the console surfaces "transaction failed, `:rollback` to recover" and sends
Bolt `RESET`/`ROLLBACK`, reusing the slice-14 error taxonomy and RESET recovery.
The open/autocommit state is shown in the prompt and status bar.

This slice covers the transaction state machine and recovery; the reconnect
interaction is issue 06.

## Acceptance criteria

- [ ] `:begin` opens a transaction; subsequent queries run within it; `:commit`
      and `:rollback` close it (committing or discarding), verified against a
      live database.
- [ ] The one-live-result guard still holds inside a transaction (a second live
      result is refused, not corrupted).
- [ ] A failing query inside a transaction poisons it and surfaces a clear
      "`:rollback` to recover" message; `:rollback` (RESET/ROLLBACK) returns the
      session to autocommit cleanly.
- [ ] `:begin`/`:commit`/`:rollback` behave identically in the REPL and
      Workbench; the open-transaction state shows in the prompt and status bar.
- [ ] A read-only session's `:begin` cannot run writes (issue 04 access mode is
      applied to the explicit transaction).

## Blocked by

None - can start immediately.
