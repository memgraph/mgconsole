# 02 — Tracer bullet: workspace + connect + run one scalar

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The first end-to-end path that every later slice builds on: a two-crate
workspace (`core` library + binary Frontend per ADR 0002), a single tokio
runtime, and a Session that connects to Memgraph by host and port, runs one
query, and returns a `QueryResult`. The binary pipes a query in, runs it, prints
a scalar as plain tabular text, and exits.

This slice also fixes two load-bearing shapes so later slices fill them
additively rather than migrating signatures (ADR 0003, ADR 0004):

- **The Core Value model and its translation seam.** The Session translates each
  `bolt_proto::Value` into a Core-owned `Value` at the Bolt boundary. The `Value`
  type is total from the outset (a variant per Memgraph type the ADR-0001 spike
  found, including `Value::Enum`); only scalars are *rendered* here, but no raw
  form is ever passed through.
- **The `QueryResult` shape.** `QueryResult { header, records: RecordStream,
  summary }`, where `RecordStream` is a Core-owned stream type (not an exposed
  `futures::Stream`) and `summary` (timing/notifications/stats from the trailing
  Bolt `SUCCESS`) is readable only after the records are drained. This slice
  populates `header` plus the one record and leaves `summary` empty; issue 13
  fills `summary`, issue 21 exploits the streaming records.

Also establishes the integration test harness: a `memgraph/memgraph` container
started on demand via `testcontainers`, so a self-contained `cargo test` needs
only a Docker daemon (no CI wiring for now). This is the harness slices 12–14,
25–26 and 30–32 reuse.

Keep the seam strict: the `core` Session knows nothing about the Frontend.

## Acceptance criteria

- [ ] Workspace builds with a `core` lib crate and a binary crate; the binary depends on `core`, not vice versa
- [ ] A single tokio runtime is created; the Session is called via `block_on` at the boundary
- [ ] `core` exposes a Session that connects by host/port and runs a query, returning a `QueryResult` (header + `RecordStream` + empty `summary` slot)
- [ ] The Session translates `bolt_proto::Value` into a Core-owned `Value` at the boundary; `Value` is total (a variant per spike-confirmed type, incl. `Value::Enum`) though only scalars are rendered here
- [ ] `RecordStream` is a Core-owned type; `futures::Stream` is not exposed in `core`'s public API
- [ ] The binary runs a query (from stdin or a hardcoded seed) and prints a scalar result
- [ ] `testcontainers` spins up `memgraph/memgraph`; an integration test connects, runs `RETURN 1`, and asserts the value
- [ ] `cargo test` runs the integration test against the container locally (Docker daemon only; no CI wiring)
- [ ] Builds as a static binary with no C toolchain or OpenSSL dependency

## Blocked by

- `.scratch/rust-console/issues/01-bolt-fidelity-spike.md`
