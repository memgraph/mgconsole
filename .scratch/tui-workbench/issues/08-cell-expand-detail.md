# 08 — Cell-expand detail view

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Let a user read a whole Value that does not fit a table cell. Selecting a cell
and triggering expand opens a detail overlay showing the full Value —
node / relationship / path / map / list — pretty-printed via the Core's existing
Value rendering. Dismissing the overlay returns to the table with selection
intact.

## Acceptance criteria

- [ ] A selected results cell can be expanded into a detail overlay showing the
      full Value, rendered by the Core's per-Value renderer.
- [ ] Node / relationship / path / map / list Values render readably in the
      overlay (multi-line, not truncated).
- [ ] Dismissing the overlay restores the table view and the prior selection.
- [ ] Overlay open/close and selection state are covered at the reducer seam.

## Blocked by

- `.scratch/tui-workbench/issues/03-streaming-results-table.md`
