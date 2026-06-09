# 29 — Connection pool for parallel import

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

A small pool of Bolt connections (e.g. `deadpool`) for the batched-parallel
import: since a Session is a single stream, N concurrent workers need N
connections. The pool creates, hands out, and recycles authenticated
connections (respecting the same host/port/auth/TLS config as a single Session).
Resolve the pool crate choice (`deadpool` vs `bb8`). Integration tested against
a container.

## Acceptance criteria

- [ ] The pool establishes N authenticated connections using the same connection config as a single Session
- [ ] Connections are handed out and returned/recycled across many tasks
- [ ] Pool size is bounded to the configured worker count
- [ ] A broken connection is replaced rather than poisoning the pool
- [ ] Integration test exercises concurrent checkout/return against a container

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
