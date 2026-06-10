# Architecture deepening — reduce primitive obsession & duplication

Status: ready-for-agent

## Why

A post-build architecture review (brief: *reduce common patterns, deduplicate,
reduce primitive obsession*) surfaced four deepening opportunities. Each turns a
shallow, repeated pattern into a small deep module with a single place to change
and test. They are independent in logic and were grilled to a fixed design; this
PRD records the decisions so each issue can be picked up unattended.

The review also flagged the four exhaustive `Value` matches (render/jsonl/csv/
cypherl) as the loudest apparent duplication, and **deliberately left them
alone**: ADR-0003 mandates wildcard-free per-renderer matches so a new variant is
a compile error at every site. Do not consolidate them.

## The four changes

1. **Typed-key Bolt metadata accessor** — `issues/01-meta-typed-key-accessor.md`
2. **Endpoint cohesion type** — `issues/02-endpoint-host-port.md`
3. **Error wrap constructors** — `issues/03-error-wrap-constructors.md`
4. **Header type at the format seam** — `issues/04-header-format-seam.md`

## Coordination

Issues 01, 02, 03 all touch `core/src/session.rs` and/or `core/src/result.rs`;
03 overlaps both. They do not logically depend on each other, but applying them
concurrently in separate worktrees will conflict textually. Apply them
sequentially (any order), rebasing each on the last. Issue 04 is isolated to
`core/src/format/` + `core/src/tabular.rs` and can run independently.

## Constraints

- No CI / packaging work (project runs local tests only).
- `unsafe` is forbidden workspace-wide (ADR-0001).
- Each change keeps existing behaviour; existing tests must still pass, and new
  unit tests cover the new module's interface.
