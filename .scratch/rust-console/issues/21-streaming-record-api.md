# 21 — Streaming record API in core

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The Core capability that makes streaming output possible (ADR 0002): the Session
exposes Records as a stream that a Frontend can consume one at a time, rather
than only as a fully-buffered collection. Buffered consumers (tabular) collect
the stream; streaming consumers (slices 22–24) write as Records arrive. Memory
stays bounded for the streaming path. Integration tested against a container
returning many rows.

## Acceptance criteria

- [x] The Session exposes Records as a consumable stream
- [x] A consumer can render Records incrementally without holding them all
- [x] A buffered consumer can still collect the full result (for tabular)
- [x] Memory stays bounded while streaming a large result (asserted, e.g. via a large generated result)
- [x] Integration test streams a many-row result from a container

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
