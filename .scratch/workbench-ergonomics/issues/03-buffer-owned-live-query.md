# 03 — Buffer-owned live query: switch-to-view while running

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics/PRD.md`

## What to build

Make a live query belong to the Buffer it was submitted from (`CONTEXT.md`
"Buffer"). Stamp the running query with its origin Buffer so its Records,
completion, and failure stream into that Buffer's Result history even when another
Buffer is active. Switching Buffers (the nav gestures, a tab-bar click) is then
allowed *while a query runs* — the user can read another line of inquiry without
cancelling — and only a *second submit* is refused (the one-live-result guard
stays session-wide, ADR 0005). A running indicator is visible for a background
Buffer's query so its completion isn't missed.

Today the running query streams into `history.last_mut()` of the *active* Buffer,
which forces "refuse to switch while running"; this slice removes that workaround.

## Acceptance criteria

- [ ] A query's Records / completion / failure land in the Buffer it was submitted
      from, regardless of which Buffer is active when they arrive.
- [ ] Buffer navigation (next/prev/new/tab-click) is allowed while a query runs; a
      second submit is still refused with the busy status.
- [ ] Switching to a Buffer whose query is still running shows it as running;
      switching back shows the streamed result once complete.
- [ ] A lifecycle event for a non-active Buffer never mutates the active Buffer's
      Result history.
- [ ] Reducer tests cover routing to a non-active Buffer and the still-refused
      second submit.

## Blocked by

None - can start immediately.
