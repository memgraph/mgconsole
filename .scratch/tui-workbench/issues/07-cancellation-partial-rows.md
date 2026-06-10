# 07 — Cancellation (Ctrl-C → RESET) + partial-rows-labelled

Status: done

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

- [x] Ctrl-C during a query cancels it and the Session survives, ready for the
      next query (RESET recovery, ADR 0005). (`interrupt` emits `Effect::Cancel`;
      the edge aborts the task — dropping the stream — so the next run RESETs.)
- [x] Rows streamed before the cancel stay on screen, scrollable; the result is
      clearly labelled partial with its count. (`CurrentResult::partial`; the pane
      title and status both show "partial" and the count; rows are kept.)
- [x] A running indicator (spinner / elapsed) is shown while a query is in
      flight and clears on completion or cancel. (braille spinner advanced by a
      120ms `Tick`, shown only while `Running`.)
- [x] The cancel-event handling (state transition + partial label) is covered at
      the reducer seam. (cancel/partial, batch-drop, idle-abandon, spinner tests.)

## Blocked by

- `.scratch/tui-workbench/issues/03-streaming-results-table.md`
