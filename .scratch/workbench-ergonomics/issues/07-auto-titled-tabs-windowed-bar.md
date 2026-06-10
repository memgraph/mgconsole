# 07 — Auto-titled tabs + windowed tab bar

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics/PRD.md`

## What to build

Give each Buffer tab an identity and make the tab bar scale. A tab is **auto-
titled** from a short snippet of its content — the originating query of its shown
Result-history entry, falling back to the first non-empty editor line — truncated
to a small fixed width, with the active tab highlighted. When the tabs exceed the
bar width, the bar **windows** (scrolls) to keep the active tab visible, with an
overflow indicator; the mouse hit-test maps a click to the correct Buffer under
windowing. Tabs stay ephemeral and auto-titled — not user-nameable (no new Buffer
state).

## Acceptance criteria

- [ ] Each tab shows a truncated title from its query snippet (fallback: first
      non-empty editor line); the active tab is highlighted.
- [ ] With more tabs than fit the width, the bar windows to keep the active tab
      visible and shows an overflow indicator.
- [ ] A tab-bar click selects the correct Buffer under windowing (the hit-test
      accounts for the scroll offset and variable title widths).
- [ ] Reducer/draw tests cover titling, windowing to the active tab, and the click
      mapping.

## Blocked by

None - can start immediately.
