# 08 — Tabular layout: column sizing, fit-to-screen, row-cap warning

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The full tabular renderer that arranges rendered Values into an aligned table:
header, per-column width sizing from the buffered rows, optional fit-to-screen
width fitting, and the row-cap behaviour from ADR 0002 — past a configurable
limit, tabular warns and points the user to a streaming format. May use a
buffer-all table crate (e.g. `comfy-table`) for the buffered path.

## Acceptance criteria

- [x] Columns are sized to the widest rendered value per column and aligned
- [x] Header row renders above the data
- [x] fit-to-screen fits the table to a given terminal width
- [x] Exceeding the row cap emits a warning and suggests a streaming format, without buffering past the cap unboundedly
- [x] Golden fixtures cover sizing, fit-to-screen on/off, and the cap warning

## Blocked by

- `.scratch/rust-console/issues/04-render-list-map.md`
- `.scratch/rust-console/issues/05-render-node-rel-path.md`
- `.scratch/rust-console/issues/06-render-temporal.md`
- `.scratch/rust-console/issues/07-render-point-enum.md`
