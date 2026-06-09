# 05 — Render nodes + relationships + paths (tabular)

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Tabular rendering of graph-structural Values: nodes (labels + properties),
relationships and unbound relationships (type + properties), and paths
(alternating nodes and relationships). Extends the rendering seam and golden
harness from slice 03. Pure function, no database.

## Acceptance criteria

- [ ] Nodes render with labels and properties
- [ ] Relationships render with type and properties; unbound relationships render correctly
- [ ] Paths render as their alternating node/relationship sequence with direction
- [ ] Property values reuse scalar/list/map rendering from slices 03–04
- [ ] Golden fixtures cover the above; tests are pure

## Blocked by

- `.scratch/rust-console/issues/03-render-scalar-string.md`
