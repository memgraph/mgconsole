# 03 — Render scalars + strings (tabular) + golden harness

Status: done

## Tabular conventions (defined here; mgconsole goldens not available in this repo)

- **null** → `Null` (explicit, so it is distinct from an empty string).
- **bool** → `true` / `false`.
- **integer** → decimal.
- **float** → always carries a decimal point so it is distinct from an integer
  (`3.0`, not `3`); `NaN` / `Inf` / `-Inf` for non-finite.
- **string** → content with control whitespace escaped (`\\`, `\n`, `\r`, `\t`)
  so it stays on one tabular line; quotes are left literal (quote-escaping is a
  CSV concern, slice 22).

Harness: `core/tests/golden/mod.rs::check_tabular(category, cases)` compares
rendered cases against `core/tests/golden/tabular/<category>.txt`; regenerate
with `UPDATE_GOLDEN=1 cargo test`. Slices 04–07 add categories.

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The Value rendering seam and its golden-file test harness, starting with scalar
Values: null, boolean, integer, float, and string (with correct escaping of
quotes and whitespace). Rendering is a pure function from a Value to its tabular
text — no database, no session.

Port the golden-file style from `mgconsole`'s `tests/input_output/`: an input
fixture of Values paired with an expected tabular output, so later type-group
slices (04–07) just add fixtures.

## Acceptance criteria

- [x] A pure rendering function turns a Value into its tabular text representation
- [x] null, bool, integer, float render correctly
- [x] strings render with correct escaping of quotes and whitespace
- [x] A golden-file harness compares rendered output to expected fixtures and is reusable by later rendering slices
- [x] Tests are pure (no container, no Session)

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
