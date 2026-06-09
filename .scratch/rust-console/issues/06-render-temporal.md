# 06 — Render temporal values (tabular)

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Tabular rendering of the full Memgraph temporal family: date, local time, local
datetime, duration, and zoned datetime. Extends the rendering seam and golden
harness from slice 03. Pure function, no database. The temporal crate is
**`chrono`** — decided by the ADR-0001 spike, since that is what `bolt-proto`
exposes (`NaiveDate`, `NaiveTime`, `NaiveDateTime`, `DateTime<FixedOffset>`,
`DateTime<Tz>` via `chrono-tz`, and a `Duration` struct).

The spike found a zoned datetime arrives as **two distinct Core `Value` arms**,
both of which must render: `DateTimeOffset` (a fixed UTC offset, e.g. `+02:00`)
and `DateTimeZoned` (a named IANA zone, e.g. `Europe/Zagreb`).

## Acceptance criteria

- [ ] date, local time, local datetime render correctly
- [ ] duration renders correctly
- [ ] both zoned-datetime arms render: `DateTimeOffset` (fixed offset) and `DateTimeZoned` (named IANA zone)
- [ ] Rendering matches Memgraph's textual conventions for these types
- [ ] Golden fixtures cover the above; tests are pure (note: Memgraph emits fractional seconds with exactly 3 or 6 digits — fixtures should reflect that)

## Blocked by

- `.scratch/rust-console/issues/03-render-scalar-string.md`
