# 06 — State-dependent reconnect inside a transaction

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Make reconnect depend on the Session's Transaction state (ADR 0011). Today a
dropped connection is re-established silently (slice 14). That stays true in
autocommit. But **in an open transaction a connection loss is not silently
retried** — it surfaces as a transaction-aborted error and drops the Session back
to autocommit, because the uncommitted work is gone and silently reconnecting
would falsely imply the transaction survived. The guiding rule: the console never
silently re-runs or resurrects work the user has explicitly bracketed.

## Acceptance criteria

- [x] In autocommit, a dropped connection still reconnects silently (slice-14
      behaviour unchanged).
- [x] In an open transaction, a connection loss surfaces a clear
      transaction-aborted error and returns the Session to autocommit, with no
      silent retry of the in-flight work.
- [x] The reconnect observer hook still fires for the autocommit path; the
      transactional path reports abortion rather than a reconnect attempt.
- [x] Covered at the Session seam with a TCP proxy that severs the connection in
      each state (autocommit reconnect test already existed; new test covers the
      open-transaction abort).

## Blocked by

- `.scratch/console-ergonomics/issues/05-explicit-transactions-state.md`

## Comments

Implemented (AFK), all in `core/src/session.rs`.

- The query path's `Error::Connection` handling is now state-dependent: in
  autocommit it reconnects (bounded) and retries once (unchanged, observer fires);
  in `Open`/`Failed` it sets `tx = Auto` and returns the new `Error::TransactionAborted`
  **without** reconnecting or retrying — the next autocommit query reconnects
  lazily, so the observer fires there, not on the abort.
- `commit` on a transport failure → `TransactionAborted` (uncommitted work gone);
  `rollback` on a transport failure → `Ok` (the tx is already discarded, the goal
  is met); `begin` reconnects+retries once (autocommit-state work, nothing
  bracketed at stake yet) via an extracted `begin_once`.
- New integration test (`connection_loss_in_a_transaction_aborts_to_autocommit`)
  uses the existing severable TCP proxy: begin → cut → next query aborts to
  autocommit with the observer un-fired, then a following autocommit query
  reconnects (observer fires).
- The Frontends needed no change: they already surface the error and sync the
  `[tx]` marker after every query (REPL `sync_tx`, Workbench `TransactionApplied`),
  so an abort flips the marker back to autocommit automatically.
