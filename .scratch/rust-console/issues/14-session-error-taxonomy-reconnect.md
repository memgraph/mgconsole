# 14 — Error taxonomy + reconnect-with-retry

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The Session's error model and recovery: distinguish a recoverable query error
(bad Cypher, constraint violation — report and keep the Session) from a fatal
connection error (socket dropped — trigger reconnect). On a fatal error the
Session reconnects with bounded retries before giving up. Integration tested by
inducing both error classes against a container.

**The `TransientError` tier is not a reliable signal.** The ADR-0001 spike found
Memgraph wrapping a plain, permanent validation error (a malformed
`localDateTime` literal) in a `Memgraph.TransientError.MemgraphError.MemgraphError`
code. Classification must therefore key off the **full / leaf error code**, not
the `TransientError` substring — otherwise a permanent user error reads as
retryable. Prefer matching the specific leaf code against a small known set
rather than the broad tier.

Also implements the **one-live-result guard** (ADR 0005): `run()` returns
`Error::ResultStillOpen` if a previous result is still live, and recovers from an
abandoned result (its `RecordStream` dropped before drain) by sending Bolt
`RESET` before the new query. A drained/`discard`ed stream clears the guard.

## Acceptance criteria

- [ ] Query errors are distinguished from fatal connection errors in the Core's error type
- [ ] Classification keys off the full/leaf error code, not the `TransientError` tier (a `TransientError` code may be a permanent error)
- [ ] A query error leaves the Session usable for the next query
- [ ] A fatal error triggers reconnect with a bounded number of retries
- [ ] Exhausting retries surfaces a clear terminal failure
- [ ] `run()` errors (`ResultStillOpen`) when a prior result is still live; recovers via `RESET` from an abandoned one (ADR 0005)
- [ ] Integration tests induce a query error and a connection drop and assert the respective behaviour

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
