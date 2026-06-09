# 02 — Tracer bullet: workspace + connect + run one scalar

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The first end-to-end path that every later slice builds on: a two-crate
workspace (`core` library + binary Frontend per ADR 0002), a single tokio
runtime, and a Session that connects to Memgraph by host and port, runs one
query, and returns its Records. The binary pipes a query in, runs it, prints a
scalar as plain tabular text, and exits.

Also establishes the integration test harness: a `memgraph/memgraph` container
started on demand via `testcontainers`, and CI wired to run `cargo test` with a
Docker daemon. This is the harness slices 12–14, 25–26 and 30–32 reuse.

Keep the seam strict: the `core` Session knows nothing about the Frontend.

## Acceptance criteria

- [ ] Workspace builds with a `core` lib crate and a binary crate; the binary depends on `core`, not vice versa
- [ ] A single tokio runtime is created; the Session is called via `block_on` at the boundary
- [ ] `core` exposes a Session that connects by host/port and runs a query, returning a header and Records
- [ ] The binary runs a query (from stdin or a hardcoded seed) and prints a scalar result
- [ ] `testcontainers` spins up `memgraph/memgraph`; an integration test connects, runs `RETURN 1`, and asserts the value
- [ ] CI runs the integration test against the container
- [ ] Builds as a static binary with no C toolchain or OpenSSL dependency

## Blocked by

- `.scratch/rust-console/issues/01-bolt-fidelity-spike.md`
