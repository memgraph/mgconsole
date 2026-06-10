# 11 — Overlay scroll indicators (help + cell-detail)

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics/PRD.md`

## What to build

Show when an overlay has more content than fits. The `:help` overlay and the
cell-detail overlay are scrollable but give no cue that there is more above or
below, so the user scrolls blind. Add a scroll indicator — a scrollbar, or
`▲`/`▼ more` markers — reflecting scroll position, so it's clear when and which
way to scroll. Pure presentation.

## Acceptance criteria

- [ ] The help and cell-detail overlays show an indicator when content extends
      beyond the visible area, and none when it all fits.
- [ ] The indicator reflects position (more-above and/or more-below).
- [ ] A draw test covers the indicator on an overflowing overlay and its absence
      when content fits.

## Blocked by

None - can start immediately.
