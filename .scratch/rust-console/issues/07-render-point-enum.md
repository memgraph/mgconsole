# 07 — Render spatial points + enums (tabular)

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Tabular rendering of the remaining Memgraph-specific Values: spatial points (2D
and 3D, both coordinate systems / SRIDs) and enums. These are the types most
likely to have needed codec extension in the slice-01 spike, so rendering must
match whatever the decoded representation is. Pure function, no database.

## Acceptance criteria

- [ ] 2D and 3D points render with their coordinates and coordinate system
- [ ] enums render with their qualified value
- [ ] Rendering aligns with the decoded representation established by the slice-01 spike / codec extensions
- [ ] Golden fixtures cover the above; tests are pure

## Blocked by

- `.scratch/rust-console/issues/03-render-scalar-string.md`
