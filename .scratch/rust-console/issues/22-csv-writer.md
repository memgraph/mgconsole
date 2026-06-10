# 22 — CSV writer (streaming)

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

A streaming CSV output format: write a header row then each Record as it arrives
over the streaming API (slice 21), with bounded memory. Honour the csv options —
delimiter, escape character, and doublequote behaviour (when doublequote is off,
the escape character prefixes embedded quotes). Pure writer logic with
golden-file tests; uses the `csv` crate.

## Acceptance criteria

- [x] Header and rows are written in CSV, streamed row-by-row
- [x] Configurable delimiter is honoured
- [x] doublequote on/off behaves correctly; escape character is used when doublequote is off
- [x] Values containing delimiters, quotes, and newlines are quoted/escaped correctly
- [x] Golden fixtures cover the option combinations; memory stays bounded

## Blocked by

- `.scratch/rust-console/issues/21-streaming-record-api.md`
- `.scratch/rust-console/issues/03-render-scalar-string.md`
