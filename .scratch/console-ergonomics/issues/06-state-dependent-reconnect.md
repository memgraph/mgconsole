# 06 — State-dependent reconnect inside a transaction

Status: ready-for-agent

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

- [ ] In autocommit, a dropped connection still reconnects silently (slice-14
      behaviour unchanged).
- [ ] In an open transaction, a connection loss surfaces a clear
      transaction-aborted error and returns the Session to autocommit, with no
      silent retry of the in-flight work.
- [ ] The reconnect observer hook still fires for the autocommit path; the
      transactional path reports abortion rather than a reconnect attempt.
- [ ] Covered at the Session seam with a fake that injects a connection loss in
      each state.

## Blocked by

- `.scratch/console-ergonomics/issues/05-explicit-transactions-state.md`
