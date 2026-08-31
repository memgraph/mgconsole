# 13 — Schema sidebar drawer + refresh

Status: done

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

A browsable view of the Schema (slice 12). A toggleable sidebar drawer lists the
labels, relationship types, and property keys the database holds, with a manual
refresh. The drawer is **absent** when the server schema feature is off (there is
nothing to browse), consistent with the silent static-only degradation.

## Acceptance criteria

- [x] A keybind toggles a sidebar drawer listing labels, relationship types, and
      property keys from the fetched Schema. (Ctrl-B toggles `drawer`;
      `draw_sidebar` lists the three sections; render test asserts the contents.)
- [x] A refresh action re-fetches and updates the drawer (shared with the
      completion source's refresh). (Ctrl-R → `FetchSchema` → `set_schema`; the
      sidebar reads `state.schema`, so it updates with completion.)
- [x] The drawer does not appear when the schema feature is unavailable.
      (`toggle_schema_sidebar` only opens when `state.schema.is_some()`.)
- [x] Drawer visibility and contents are covered at the reducer seam.
      (toggle-open/close and unavailable-no-open tests; sidebar render test.)

## Blocked by

- `.scratch/tui-workbench/issues/12-live-schema-completion-source.md`
