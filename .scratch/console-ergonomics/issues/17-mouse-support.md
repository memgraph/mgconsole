# 17 — Mouse support (Workbench)

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add mouse interaction to the Workbench: click to focus a pane, click a result
cell to select it (feeding the existing cell-expand detail, slice 08 of
tui-workbench), scroll to move through results and the editor, and click the tab
bar to switch Buffers (issue 18). Mouse events are translated into the existing
reducer events so behaviour stays consistent with the keyboard gestures. Openly
Workbench-only.

## Acceptance criteria

- [ ] Clicking a pane focuses it; clicking a result cell selects it and drives the
      existing cell-expand path.
- [ ] Scroll wheel moves through result rows and the editor viewport.
- [ ] Clicking the tab bar switches Buffers (when issue 18 is present); absent
      tabs, the tab bar is simply not drawn.
- [ ] Mouse events map onto existing reducer events (no parallel state path),
      covered by reducer-level tests.
- [ ] Mouse capture does not break terminal copy/paste expectations (documented
      modifier or toggle to fall back to native selection).

## Blocked by

None - can start immediately.
