# 32 — Retry-with-backoff on serialization conflicts

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The final correctness guarantee for batched-parallel import: detect Batches that
fail with a serialization/conflict error and retry them with backoff, so
transient conflicts (common in transactional mode under concurrency) do not fail
the import. Confirm overall behaviour across both storage modes — analytical
(ordering-driven, slice 31) and transactional (conflict-driven, this slice) —
against a container.

Retry must match the **specific serialization-conflict leaf code**, not the broad
`Memgraph.TransientError.*` family (per slice 14: the spike found a permanent
validation error also carrying a `TransientError` code). Retrying on the tier
would retry non-retryable errors to exhaustion. Confirm the exact conflict
leaf-code against a live container when implementing.

## Acceptance criteria

- [ ] A Batch failing with the specific serialization/conflict error code is retried rather than aborted
- [ ] A Batch failing with a non-conflict error (even under the `TransientError` tier) is *not* retried
- [ ] Retries use backoff and a bounded attempt count
- [ ] A Batch that keeps failing past the limit surfaces a clear error
- [ ] Integration test induces conflicts in transactional mode and asserts the import completes correctly
- [ ] End-to-end parallel import is verified in both analytical and transactional modes

## Blocked by

- `.scratch/rust-console/issues/31-vertices-first-ordering.md`
