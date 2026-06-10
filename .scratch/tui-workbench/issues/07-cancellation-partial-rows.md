# 07 — Cancellation (Ctrl-C → RESET) + partial-rows-labelled

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

The headline interaction (PRD story 44, now realised). Ctrl-C while a query is in
flight cancels it via Bolt `RESET`, recovering the Session per ADR 0005 and
keeping it ready for the next query. The rows that already streamed into the
table **remain on screen**, and the result is labelled "partial" with its count
(e.g. "cancelled — 12,340 rows (partial)"). A running query also shows a
running/elapsed indicator while in flight.

## Acceptance criteria

- [ ] Ctrl-C during a query cancels it and the Session survives, ready for the
      next query (RESET recovery, ADR 0005).
- [ ] Rows streamed before the cancel stay on screen, scrollable; the result is
      clearly labelled partial with its count.
- [ ] A running indicator (spinner / elapsed) is shown while a query is in
      flight and clears on completion or cancel.
- [ ] The cancel-event handling (state transition + partial label) is covered at
      the reducer seam.

## Blocked by

- `.scratch/tui-workbench/issues/03-streaming-results-table.md`
