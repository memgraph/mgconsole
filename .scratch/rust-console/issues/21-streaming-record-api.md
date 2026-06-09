# 21 — Streaming record API in core

Status: ready-for-agent

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

- [ ] The Session exposes Records as a consumable stream
- [ ] A consumer can render Records incrementally without holding them all
- [ ] A buffered consumer can still collect the full result (for tabular)
- [ ] Memory stays bounded while streaming a large result (asserted, e.g. via a large generated result)
- [ ] Integration test streams a many-row result from a container

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
