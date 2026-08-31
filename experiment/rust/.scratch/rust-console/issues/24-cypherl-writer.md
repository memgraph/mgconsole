# 24 — cypherl writer (streaming)

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

A streaming cypherl output format: write results as replayable Cypher
statements, one logical query at a time, as Records arrive with bounded memory.
This is the format that closes the export/import loop (used by slice 26's
`DUMP DATABASE` export). Pure writer logic with golden-file tests.

## Acceptance criteria

- [x] Records are written as replayable cypherl statements, streamed
- [x] Output streams row-by-row with bounded memory
- [x] Emitted cypherl re-imports cleanly (round-trips) via the serial import path
- [x] Value literals are escaped so the emitted Cypher is valid
- [x] Golden fixtures cover representative results

## Blocked by

- `.scratch/rust-console/issues/21-streaming-record-api.md`
- `.scratch/rust-console/issues/03-render-scalar-string.md`
