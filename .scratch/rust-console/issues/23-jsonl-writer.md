# 23 — JSONL writer (streaming)

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

A streaming JSONL output format: emit one JSON object per Record (keyed by
column name) as Records arrive, with bounded memory. This is the new format
beyond today's tool, aimed at piping into tools like jq. Define and test the
JSON shape for each Value type (how nodes, relationships, paths, temporal,
point, and enum Values serialise). Uses serde / serde_json; pure writer logic
with golden-file tests.

## Acceptance criteria

- [x] Each Record is emitted as one JSON object on its own line, keyed by column
- [x] Output streams row-by-row with bounded memory
- [x] A defined, documented JSON encoding exists for every Value type (graph, temporal, spatial, enum)
- [x] Output is valid JSONL consumable by a downstream parser
- [x] Golden fixtures cover each Value type

## Blocked by

- `.scratch/rust-console/issues/21-streaming-record-api.md`
- `.scratch/rust-console/issues/03-render-scalar-string.md`
