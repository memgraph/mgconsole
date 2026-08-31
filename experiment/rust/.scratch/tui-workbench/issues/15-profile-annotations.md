# 15 — PROFILE annotations on the plan tree

Status: done

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Make `PROFILE` actionable for tuning. On top of the plan tree (slice 14),
annotate each operator with the actual execution statistics Memgraph returns for
a `PROFILE` query — hits and timing per operator — so a writer can see where a
query spends its work. An `EXPLAIN` plan (no stats) renders as before, without
annotations.

## Acceptance criteria

- [x] A `PROFILE` query's plan tree shows per-operator hits/time annotations.
      (`plan::annotation` builds "N hits · rel% · abs ms" from the extra columns;
      `draw_plan` trails it dimmed after each operator.)
- [x] An `EXPLAIN` plan renders without annotations (no empty/zero columns).
      (an `EXPLAIN` row has only the operator column, so `annotation` is `None`.)
- [x] The annotation parsing/attachment is covered by unit tests; a thin
      `TestBackend` render asserts annotated nodes. (`a_profile_row_carries_...`
      + EXPLAIN-no-annotation unit tests; `renders_a_profile_plan_with_annotations`
      golden.)

## Blocked by

- `.scratch/tui-workbench/issues/14-explain-plan-tree.md`
