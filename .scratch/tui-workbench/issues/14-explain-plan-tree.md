# 14 — EXPLAIN plan tree (polymorphic results pane)

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Render a query plan as the tree it is. A pure predicate over the leading lexer
token detects an `EXPLAIN` (or `PROFILE`) statement; for such a query the results
pane switches to **tree mode**, rendering the returned plan as a navigable,
collapsible operator tree instead of a flat table. For an ordinary query the pane
stays in table mode. The plan appears where results appear — it is the output of
what the user ran.

PROFILE's per-operator hits/time annotations are slice 15; this slice delivers
the tree structure (and renders a PROFILE plan's tree without the annotations).

## Acceptance criteria

- [ ] A pure prefix predicate identifies `EXPLAIN`/`PROFILE` statements
      (case-insensitive, over the lexer's leading token); unit-tested.
- [ ] An `EXPLAIN` query renders its plan as a navigable, collapsible operator
      tree in the results pane; an ordinary query still renders a table.
- [ ] Tree nodes expand/collapse and navigate with the keyboard.
- [ ] Mode selection (tree vs table) and tree navigation are covered at the
      reducer seam, with a thin `TestBackend` golden render of a plan.

## Blocked by

- `.scratch/tui-workbench/issues/03-streaming-results-table.md`
