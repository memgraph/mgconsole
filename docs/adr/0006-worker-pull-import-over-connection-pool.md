# 6. Worker-pull parallel import over a connection pool

Date: 2026-06-10

## Status

Accepted (supersedes the connection-pool framing in the original issues 29/30
and the PRD's import-engine note)

## Context

Batched-parallel import distributes a **finite** set of Batches across a
**fixed** number of **long-lived** connections (a Session is a single stream, so
N workers need N connections). A connection-pool crate (`deadpool` / `bb8`)
targets the opposite shape — open-ended, bursty request/response traffic with
dynamic checkout/return and sizing.

## Decision

Use a **worker-pull** model: spawn N long-lived worker tasks, **each owning a
`Session`**, all pulling Batches from a shared queue (e.g. an `mpsc` /
`async-channel` receiver). No connection-pool crate; the `deadpool`-vs-`bb8`
question is dropped.

## Consequences

- Import reuses `Session` — its `run`, error taxonomy and reconnect (slice 14),
  and the one-live-result guard (ADR 0005) — instead of operating on raw pooled
  connections.
- No pool-size-vs-Semaphore redundancy: the worker count *is* the concurrency
  bound.
- A broken connection is handled by *that worker's* Session reconnecting, not a
  pool's replace logic.
- Vertices-first ordering (slice 31) becomes a barrier between two queue phases
  (drain node-Batches, then edge-Batches); retry-with-backoff (slice 32)
  re-runs or re-queues a conflicted Batch on its worker.
- Issue 29 is re-scoped from "connection pool" to "establish N worker Sessions";
  issue 30 distributes Batches over the worker queue.
- If a future long-running/daemon use needs dynamic pooling for many unrelated
  operations, revisit — that workload would actually fit a pool.
