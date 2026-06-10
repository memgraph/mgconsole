# 26 — `DUMP DATABASE` export → cypherl

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Close the export/import loop: running `DUMP DATABASE` and writing its result in
cypherl format produces a file that re-imports cleanly via the serial import
path (slice 25). This is mostly an integration assertion that the cypherl writer
(24) plus serial import (25) round-trip a real database. Tested against a
container seeded with data.

## Acceptance criteria

- [ ] `DUMP DATABASE` output written as cypherl captures the seeded data
- [ ] Re-importing that cypherl into an empty database reproduces the data
- [ ] The round-trip (dump → import → dump) is stable
- [ ] Integration test seeds a container, dumps, re-imports into a fresh container, and asserts equivalence

## Blocked by

- `.scratch/rust-console/issues/24-cypherl-writer.md`
- `.scratch/rust-console/issues/25-serial-import-pipe.md`
