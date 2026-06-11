# 03 — Streaming results table widget

Status: done

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

The navigable results table. Records arriving over the channel (slice 02) append
to the current result and render into a native ratatui table with a **pinned
header** and **scrollable, lazily-rendered rows** — only the visible window is
drawn, so a large result is navigable rather than refused. Each cell's text comes
from the Core's per-Value renderer; the workbench owns only the layout. A **live
row count** updates as rows stream in, and the REPL's row cap is relaxed to a
high, configurable backstop (a memory guard, not a usability limit). A result
with no columns (e.g. a write) shows just the summary.

## Acceptance criteria

- [x] Rows appear incrementally as they stream in, with a live count; the header
      stays pinned while scrolling. (`on_record` appends; count in the pane title;
      `Table::header` pins the header row.)
- [x] Only the visible window is rendered (a result far larger than the old cap
      is navigable); a high configurable backstop bounds memory.
      (`draw_result` renders only `rows[scroll..scroll+viewport]`;
      `WorkbenchConfig::row_cap` = `DEFAULT_ROW_CAP` 1_000_000.)
- [x] Cell text is produced by the Core's existing per-Value renderer; the
      whole-result table renderer used by the REPL/serial paths is untouched.
      (`render::tabular` per cell; `render_table` unchanged.)
- [x] Scrolling, paging, and column navigation work via the keyboard.
      (`results_key`: Up/Down/PageUp/PageDown/Home/End/Left/Right, clamped.)
- [x] Table state (scroll position, row count, visible window) is covered at the
      reducer seam, with a thin ratatui `TestBackend` golden render.
      (navigation/scroll-follow/row-cap reducer tests + a table render test.)

## Blocked by

- `.scratch/tui-workbench/issues/02-nonblocking-execution-spine.md`
