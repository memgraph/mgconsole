# 30 — Batched-parallel executor

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The concurrency engine of the batched-parallel Import mode: read the input into
Batches of a configurable size, and execute Batches concurrently over the
connection pool (29) using a tokio JoinSet bounded by a Semaphore set to the
worker count. Wire up the batch-size and workers-number flags. This slice
delivers raw parallel execution; correctness guarantees (ordering, retry) come
in 31 and 32. Integration tested against a container.

## Acceptance criteria

- [ ] Input is split into Batches of the configured size
- [ ] Batches run concurrently over the pool, bounded by the worker count (Semaphore)
- [ ] batch-size and workers-number flags control batching and concurrency
- [ ] All Batches complete and their effects are applied to the database
- [ ] Integration test imports a dataset in parallel and asserts the resulting data

## Blocked by

- `.scratch/rust-console/issues/25-serial-import-pipe.md`
- `.scratch/rust-console/issues/29-connection-pool.md`
