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

## Acceptance criteria

- [ ] Query errors are distinguished from fatal connection errors in the Core's error type
- [ ] A query error leaves the Session usable for the next query
- [ ] A fatal error triggers reconnect with a bounded number of retries
- [ ] Exhausting retries surfaces a clear terminal failure
- [ ] Integration tests induce a query error and a connection drop and assert the respective behaviour

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
