# 05 — Bolder transaction status indicator

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics-2/PRD.md` — makes the explicit-transaction state
impossible to miss.

## What to build

Replace the dim `[tx]` / `[tx failed]` status marker with a loud, glanceable
segment, so "you are in an explicit transaction" registers at a glance.

- While a Transaction is Open, the status bar shows a reverse-video / amber
  segment **`TX N OPEN`** (N = the Transaction episode number from issue 04); when
  the transaction is poisoned, a red **`TX N FAILED`**. Autocommit shows nothing.
- The colours come from a new **theme UI slot** (extending the issue-08 UI-slot
  theme), so they recolour with the active theme and the `light` built-in.

Demo: `:begin` → a bright `TX 2 OPEN` segment appears; a failing statement turns
it red `TX 2 FAILED`; `:rollback` clears it.

## Acceptance criteria

- [ ] An Open transaction renders a high-contrast `TX N OPEN` status segment with
      the episode number; a Failed transaction renders `TX N FAILED` in the error
      colour; autocommit renders neither.
- [ ] The segment's colours are theme UI slots, resolved over the active theme
      (including `light`).
- [ ] Draw/snapshot tests cover the open, failed, and autocommit states.

## Blocked by

- `.scratch/workbench-ergonomics-2/issues/04-transaction-episode-correlated-history.md`
