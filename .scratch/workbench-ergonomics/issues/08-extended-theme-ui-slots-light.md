# 08 — Extended theme: UI-element colours + `light` built-in

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics/PRD.md`

## What to build

Extend the theme beyond highlight categories to the Workbench chrome, and ship a
light theme. Add palette slots for the UI elements currently hardcoded — `border`
(pane/overlay borders, today `Cyan`), `selection` (the selected cell/row, today
reverse-video), and `status` (the status bar) — overridable through the existing
`[theme]` table and carried by the built-in themes. Add a **`light`** built-in
alongside `default`/`mono`, tuned for light-background terminals so e.g. comment
grey stays legible. The draw consults the palette for these elements instead of
hardcoded colours; an absent override keeps today's appearance.

## Acceptance criteria

- [ ] `[theme]` can override `border`, `selection`, and `status`; an empty/absent
      table reproduces today's appearance.
- [ ] The draw uses the palette for pane/overlay borders, the selected cell/row,
      and the status bar (no remaining hardcoded `Cyan`/reverse for these).
- [ ] `:set theme light` selects a light-tuned built-in; `:set theme` lists it
      among the built-ins.
- [ ] Palette resolution (the new slots) and a draw smoke test (borders/selection/
      status pick up an override; `light` differs from `default`) are covered.

## Blocked by

None - can start immediately.
