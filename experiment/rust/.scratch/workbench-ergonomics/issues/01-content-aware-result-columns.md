# 01 — Content-aware result columns

Status: done

## Parent

`.scratch/workbench-ergonomics/PRD.md`

## What to build

Size the Workbench results table's columns to their visible content instead of
equal `1/N` ratios, so a wide column gets the room it needs and narrow ones don't
waste it. Each column is sized to the widest visible cell (header included),
clamped to a readable minimum and a maximum of roughly half the pane; the last
column flexes to fill the remainder. A cell wider than its column truncates with a
trailing `…`, with the full Value reachable via cell-expand (Enter) and yank
(issue 04). Sizing reads only the visible row window, so a huge result stays cheap.

This is the Workbench's answer to "a row won't fit" — the display Setting
(tabular/vertical/auto) is buffered-render-only and does not apply to the
Workbench's live table (`CONTEXT.md` "Display mode").

## Acceptance criteria

- [ ] Columns size to the widest visible cell (header included), clamped to a min
      and a max (~½ the pane); the last column takes the remainder.
- [ ] A cell wider than its column is truncated with a trailing `…`; cell-expand
      still shows the Value in full.
- [ ] Sizing reads only the visible row window (a very large result stays
      responsive — no full scan per frame).
- [ ] Covered by a draw/render test: a wide column and a narrow column get
      proportional widths and the truncation marker appears.

## Blocked by

None - can start immediately.
