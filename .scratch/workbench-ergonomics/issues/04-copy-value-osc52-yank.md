# 04 — Copy a Value out: OSC 52 yank + `:set mouse off`

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics/PRD.md` — implements **ADR 0015** (clipboard via
OSC 52).

## What to build

Let the user copy a Value out of the Workbench under mouse capture. A **yank**
gesture in the results pane copies to the system clipboard via an **OSC 52**
terminal escape (ADR 0015 — pure bytes, no native clipboard dependency, works over
SSH): `y` copies the selected cell's rendered Value, `Y` copies the whole selected
row; the cell-detail overlay yanks the same way. The status line confirms what was
copied. (`Ctrl+C` is cancel, hence the vim-style `y`/`Y`.)

Separately, `:set mouse off` releases mouse capture for the session (and `:set
mouse on` re-enables it), restoring native click-drag selection as the documented
escape hatch; `Shift`-drag remains the per-action fallback. The yank copies
*rendered* text (what is shown), not a re-serialization.

## Acceptance criteria

- [ ] `y` copies the selected cell's rendered text and `Y` the selected row, each
      via an OSC 52 sequence emitted at the IO edge; the cell-detail overlay yanks
      too.
- [ ] The status line confirms what was copied.
- [ ] `:set mouse off` releases mouse capture; `:set mouse on` re-enables it; the
      setting is listed by `:set`.
- [ ] No native clipboard dependency is added (OSC 52 only; honours ADR 0001 /
      ADR 0015).
- [ ] The reducer emits a copy effect carrying the exact payload (cell vs row),
      and the mouse-capture toggle is covered by tests at the reducer/IO seam.

## Blocked by

None - can start immediately.
