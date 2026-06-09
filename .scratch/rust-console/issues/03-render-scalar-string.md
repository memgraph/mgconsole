# 03 — Render scalars + strings (tabular) + golden harness

Status: ready-for-agent

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

- [ ] A pure rendering function turns a Value into its tabular text representation
- [ ] null, bool, integer, float render correctly
- [ ] strings render with correct escaping of quotes and whitespace
- [ ] A golden-file harness compares rendered output to expected fixtures and is reusable by later rendering slices
- [ ] Tests are pure (no container, no Session)

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
