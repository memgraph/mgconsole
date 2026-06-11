# 14 — EXPLAIN plan tree (polymorphic results pane)

Status: done

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

- [x] A pure prefix predicate identifies `EXPLAIN`/`PROFILE` statements
      (case-insensitive, over the lexer's leading token); unit-tested.
      (`plan::is_plan_query`; unit tests incl. the in-string false positive.)
- [x] An `EXPLAIN` query renders its plan as a navigable, collapsible operator
      tree in the results pane; an ordinary query still renders a table.
      (`Plan::parse` on completion when `is_plan_query`; `draw_result` branches to
      `draw_plan`; tests assert plan-vs-table mode selection.)
- [x] Tree nodes expand/collapse and navigate with the keyboard. (`results_key`
      routes Up/Down/Left/Right/Enter to `Plan` nav/collapse when a plan is shown.)
- [x] Mode selection (tree vs table) and tree navigation are covered at the
      reducer seam, with a thin `TestBackend` golden render of a plan.
      (parse/visibility/navigation unit tests + plan-mode reducer tests + a
      `draw_plan` golden render.)

## Note

Verified against Memgraph 3.10.1: `EXPLAIN` → one `QUERY PLAN` column of indented
`* Operator` strings; `PROFILE` → an `OPERATOR` column (indented) plus hits/time
columns (the annotations attached in slice 15). The tree is parsed from the
operator column's leading indentation; slice 14 ignores the annotation columns.

## Blocked by

- `.scratch/tui-workbench/issues/03-streaming-results-table.md`
