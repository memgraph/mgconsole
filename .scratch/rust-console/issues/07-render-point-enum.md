# 07 — Render spatial points + enums (tabular)

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Tabular rendering of the remaining Memgraph-specific Values: spatial points (2D
and 3D, both coordinate systems / SRIDs) and enums. Pure function, no database.

The ADR-0001 spike settled the representation — **no codec extension was needed**.
Points decode to `Point2D`/`Point3D` carrying their SRID (observed: 7203/4326 for
2D cartesian/wgs84, 9157/4979 for 3D cartesian/wgs84). An enum is **not** a
transport concern: Memgraph transmits it as a tagged map, which the Session
normalises into a first-class `Value::Enum` at the boundary (ADR 0003). This
slice renders that Core `Value::Enum` as `Type::Member` (e.g. `Status::Active`);
the sentinel-map detection lives in the boundary, not here.

## Acceptance criteria

- [ ] 2D and 3D points render with their coordinates and coordinate system (SRID)
- [ ] `Value::Enum` renders as its qualified value, e.g. `Status::Active`
- [ ] Rendering matches the Core `Value` model (ADR 0003), not `bolt_proto::Value`
- [ ] Golden fixtures cover the above; tests are pure

## Blocked by

- `.scratch/rust-console/issues/03-render-scalar-string.md`
