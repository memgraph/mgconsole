# 29 — Worker Sessions for parallel import

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Establish the N long-lived connections the batched-parallel import runs over.
Per ADR 0006 this is a **worker-pull** model, **not** a connection-pool crate:
create N `Session`s up front (each its own Bolt connection) using the same
host/port/auth/TLS config as a single Session, ready to be fed Batches from a
shared queue in slice 30. A worker whose connection breaks reconnects via the
Session's own reconnect (slice 14) rather than a pool replacing it.

## Acceptance criteria

- [ ] N authenticated worker Sessions are established using the same connection config as a single Session
- [ ] The worker count is configurable and bounds the number of connections
- [ ] A worker whose connection breaks reconnects (slice 14) without aborting the others
- [ ] Integration test stands up N workers against a container and runs a query on each

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
- `.scratch/rust-console/issues/14-session-error-taxonomy-reconnect.md`
