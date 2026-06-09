# 08 — Tabular layout: column sizing, fit-to-screen, row-cap warning

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The full tabular renderer that arranges rendered Values into an aligned table:
header, per-column width sizing from the buffered rows, optional fit-to-screen
width fitting, and the row-cap behaviour from ADR 0002 — past a configurable
limit, tabular warns and points the user to a streaming format. May use a
buffer-all table crate (e.g. `comfy-table`) for the buffered path.

## Acceptance criteria

- [ ] Columns are sized to the widest rendered value per column and aligned
- [ ] Header row renders above the data
- [ ] fit-to-screen fits the table to a given terminal width
- [ ] Exceeding the row cap emits a warning and suggests a streaming format, without buffering past the cap unboundedly
- [ ] Golden fixtures cover sizing, fit-to-screen on/off, and the cap warning

## Blocked by

- `.scratch/rust-console/issues/04-render-list-map.md`
- `.scratch/rust-console/issues/05-render-node-rel-path.md`
- `.scratch/rust-console/issues/06-render-temporal.md`
- `.scratch/rust-console/issues/07-render-point-enum.md`
