# 31 — Vertices-first ordering in parallel import

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Correctness for batched-parallel import: use the clause scanner (27) to enforce
vertices-first ordering, so node-creating queries are applied before the
edge-creating queries that depend on them. Without this, concurrent import can
create edges whose endpoints do not yet exist (or, in analytical mode, silently
produce a wrong graph). Integration tested against a container in analytical
mode where ordering matters.

## Acceptance criteria

- [ ] Node-creating Batches are applied before dependent edge-creating Batches
- [ ] The scanner's clause detection drives the ordering decision
- [ ] A dataset with interleaved node/edge creates imports to a correct graph under parallelism
- [ ] Integration test in analytical mode asserts the resulting graph is correct (no missing endpoints)

## Blocked by

- `.scratch/rust-console/issues/30-parallel-executor.md`
- `.scratch/rust-console/issues/27-clause-scanner.md`
