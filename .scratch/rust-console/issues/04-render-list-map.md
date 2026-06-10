# 04 — Render lists + maps (tabular)

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Tabular rendering of list and map Values, including nesting (lists of maps, maps
of lists, arbitrary depth). Extends the rendering seam and golden harness from
slice 03 with new fixtures. Pure function, no database.

## Acceptance criteria

- [x] Lists render with their elements, including empty lists
- [x] Maps render with their key/value pairs, including empty maps
- [x] Nested combinations (list-of-map, map-of-list, deep nesting) render correctly
- [x] Values inside containers reuse the scalar/string rendering from slice 03
- [x] Golden fixtures cover the above; tests are pure

## Blocked by

- `.scratch/rust-console/issues/03-render-scalar-string.md`
