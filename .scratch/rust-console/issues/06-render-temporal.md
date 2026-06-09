# 06 — Render temporal values (tabular)

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Tabular rendering of the full Memgraph temporal family: date, local time, local
datetime, duration, and zoned datetime. Extends the rendering seam and golden
harness from slice 03. Pure function, no database. Resolve the temporal crate
choice (`time` vs `chrono`) to match what `bolt-proto` exposes.

## Acceptance criteria

- [ ] date, local time, local datetime render correctly
- [ ] duration renders correctly
- [ ] zoned datetime renders with its zone/offset
- [ ] Rendering matches Memgraph's textual conventions for these types
- [ ] Golden fixtures cover the above; tests are pure

## Blocked by

- `.scratch/rust-console/issues/03-render-scalar-string.md`
