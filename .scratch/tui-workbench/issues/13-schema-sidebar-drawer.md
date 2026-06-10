# 13 — Schema sidebar drawer + refresh

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

A browsable view of the Schema (slice 12). A toggleable sidebar drawer lists the
labels, relationship types, and property keys the database holds, with a manual
refresh. The drawer is **absent** when the server schema feature is off (there is
nothing to browse), consistent with the silent static-only degradation.

## Acceptance criteria

- [ ] A keybind toggles a sidebar drawer listing labels, relationship types, and
      property keys from the fetched Schema.
- [ ] A refresh action re-fetches and updates the drawer (shared with the
      completion source's refresh).
- [ ] The drawer does not appear when the schema feature is unavailable.
- [ ] Drawer visibility and contents are covered at the reducer seam.

## Blocked by

- `.scratch/tui-workbench/issues/12-live-schema-completion-source.md`
