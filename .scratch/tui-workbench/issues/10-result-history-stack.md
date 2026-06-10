# 10 — Result-history stack

Status: ready-for-agent

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

- [ ] Every statement's result is pushed onto a history stack; a multi-statement
      submit pushes N entries in order.
- [ ] Back/forward navigation moves between results, across statements and across
      submits; the status indicates position (e.g. "result 3 of 5").
- [ ] Each revisited result keeps its rows, summary, and partial/plan state.
- [ ] History push/navigation is covered at the reducer seam.

## Blocked by

- `.scratch/tui-workbench/issues/02-nonblocking-execution-spine.md`
