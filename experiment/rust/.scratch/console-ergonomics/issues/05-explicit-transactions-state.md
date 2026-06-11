# 05 — Explicit transactions: the Session transaction state

Status: done

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

- [x] `:begin` opens a transaction; subsequent queries run within it; `:commit`
      and `:rollback` close it (committing or discarding), verified against a
      live database.
- [x] The one-live-result guard still holds inside a transaction (a second live
      result is refused, not corrupted).
- [x] A failing query inside a transaction poisons it and surfaces a clear
      "`:rollback` to recover" message; `:rollback` (RESET/ROLLBACK) returns the
      session to autocommit cleanly.
- [x] `:begin`/`:commit`/`:rollback` behave identically in the REPL and
      Workbench; the open-transaction state shows in the prompt and status bar.
- [x] A read-only session's `:begin` cannot run writes (issue 04 access mode is
      applied to the explicit transaction).

## Blocked by

None - can start immediately.

## Comments

Implemented (AFK).

- **Core** (`core/src/session.rs`): a `TransactionState {Auto, Open, Failed}` on
  the Session with `begin`/`commit`/`rollback` and `transaction_state()`. `:begin`
  sends `BEGIN` (with `mode: r` when read-only, composing issue 04); queries run
  within the open tx (no per-query wrap); `:commit`/`:rollback` close it. A query
  `FAILURE` inside an open tx is *not* auto-RESET — the tx is poisoned (`Failed`);
  the next `run` returns `Error::TransactionFailed` until `:rollback`, which sends
  `RESET` to clear the server's FAILED state. The one-live-result guard composes
  unchanged. New error variants `TransactionFailed`/`Transaction`. Five live
  integration tests (commit visible, rollback discards, poison+recover, read-only
  tx rejects writes, one-live-result inside a tx).
- **Frontends**: `:begin`/`:commit`/`:rollback` parse into the shared
  `MetaCommand` and are honoured identically by the REPL loop (via a new
  `dispatch_meta` helper extracted to keep `run_loop` small as the vocabulary
  grows) and the Workbench reducer (via `Effect::Transaction(TxOp)` →
  `Event::TransactionApplied`). The open/failed state shows as `[tx]`/`[tx failed]`
  in the REPL prompt (shared `AtomicU8`, synced after each query so a poison is
  visible) and the Workbench status bar.
- A reconnect resets the tx to autocommit for now; issue 06 refines the
  never-silently-resurrect policy.
