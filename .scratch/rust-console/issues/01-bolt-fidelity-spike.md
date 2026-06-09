# 01 — Bolt fidelity spike (ADR-0001 go/no-go)

Status: ready-for-human

## Parent

`.scratch/rust-console/PRD.md`

## What to build

A throwaway spike that connects the pure-Rust Bolt stack (`bolt-client` +
`bolt-proto`) to a live Memgraph and proves whether every Memgraph Value type
decodes faithfully at the raw-Value layer. This is the gating decision for
ADR 0001: if the codec gap is larger than a handful of struct signatures, we
fall back to FFI to `mgclient`.

Stand up Memgraph from the official `memgraph/memgraph` Docker image, run
queries that return each Value type, and record which decode cleanly, which
arrive as unknown PackStream struct signatures, and what extending the codec
would take.

This slice ends in a written go/no-go recommendation, not production code.

## Acceptance criteria

- [ ] `bolt-client`/`bolt-proto` connects to a `memgraph/memgraph` container and runs a trivial query
- [ ] A query returns each Value type: null, bool, integer, float, string, list, map, node, relationship, unbound relationship, path, date, localtime, localdatetime, duration, zoned datetime, spatial point (2D/3D, both SRIDs), enum
- [ ] Each type is recorded as decodes-faithfully / needs-codec-extension / fails, with the observed PackStream signature for any gap
- [ ] A short written recommendation: proceed with pure-Rust (listing codec extensions needed) or invoke the `mgclient` FFI fallback
- [ ] Findings captured so ADR 0001 can be confirmed or revised

## Blocked by

- None - can start immediately
