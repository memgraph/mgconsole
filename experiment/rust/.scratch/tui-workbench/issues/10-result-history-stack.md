# 10 — Result-history stack

Status: done

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

A navigable history of results. Each statement's result is pushed onto a stack;
the results pane shows the latest, and back/forward keys revisit earlier ones —
across statements of one submit **and** across separate submits. A multi-statement
submit pushes one entry per statement. This is the natural home for the
multi-statement case (PRD story 11) and gives "scroll back to my previous query's
result," which the REPL cannot offer.

## Acceptance criteria

- [x] Every statement's result is pushed onto a history stack; a multi-statement
      submit pushes N entries in order. (`on_started` pushes one `CurrentResult`
      per statement; test asserts N entries.)
- [x] Back/forward navigation moves between results, across statements and across
      submits; the status indicates position (e.g. "result 3 of 5"). (`[` / `]`
      move `view`, clamped; the results pane title shows `[N/M]`.)
- [x] Each revisited result keeps its rows, summary, and partial/plan state.
      (each entry is its own `CurrentResult` with its own rows/scroll/summary;
      test confirms an older result's rows survive.)
- [x] History push/navigation is covered at the reducer seam. (push-per-statement,
      back/forward-clamped, and revisited-rows tests.)

## Blocked by

- `.scratch/tui-workbench/issues/02-nonblocking-execution-spine.md`
