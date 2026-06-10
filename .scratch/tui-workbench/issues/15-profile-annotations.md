# 15 — PROFILE annotations on the plan tree

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Make `PROFILE` actionable for tuning. On top of the plan tree (slice 14),
annotate each operator with the actual execution statistics Memgraph returns for
a `PROFILE` query — hits and timing per operator — so a writer can see where a
query spends its work. An `EXPLAIN` plan (no stats) renders as before, without
annotations.

## Acceptance criteria

- [ ] A `PROFILE` query's plan tree shows per-operator hits/time annotations.
- [ ] An `EXPLAIN` plan renders without annotations (no empty/zero columns).
- [ ] The annotation parsing/attachment is covered by unit tests; a thin
      `TestBackend` render asserts annotated nodes.

## Blocked by

- `.scratch/tui-workbench/issues/14-explain-plan-tree.md`
