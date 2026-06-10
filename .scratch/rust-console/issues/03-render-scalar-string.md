# 03 — Render scalars + strings (tabular) + golden harness

Status: done

## Tabular conventions (defined here; mgconsole goldens not available in this repo)

- **null** → `Null` (explicit, so it is distinct from an empty string).
- **bool** → `true` / `false`.
- **integer** → decimal.
- **float** → always carries a decimal point so it is distinct from an integer
  (`3.0`, not `3`); shortest representation that round-trips to the same `f64`;
  `NaN` / `Inf` / `-Inf` for non-finite.
- **string** → content with control whitespace escaped (`\\`, `\n`, `\r`, `\t`)
  so it stays on one tabular line; quotes are left literal (quote-escaping is a
  CSV concern, slice 22).

**Floats and null deliberately diverge from Memgraph's `toString`** (unlike the
temporal and point slices, which match it). `toString(1.0)` is `1`, which
conflates a float with an integer — the opposite of faithful type display — and
`toString` on a non-round float prints the noisy exact-binary-decimal expansion
(`123456789.123456791043282`) rather than the shortest round-tripping form
(`...79`). `toString` is Memgraph's string-*coercion* semantics, not a display
convention, so we keep our renderer: floats always show their decimal point and
round-trip; `null` renders as `Null`, not empty. Temporals/points still match
`toString` because there those forms are the natural display and lose nothing.

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
