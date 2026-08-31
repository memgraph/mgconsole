# 11. Explicit transactions add a Session state; reconnect becomes state-dependent

Date: 2026-06-10

## Status

Accepted

## Context

The Session has been pure autocommit: every query is its own implicit
transaction, and a dropped connection is re-established *silently* (slice 14
reconnect). Adding `:begin`/`:commit`/`:rollback` lets several queries share one
multi-statement transaction. That breaks an assumption silent reconnect relied
on — that any one query can be safely re-run on a fresh connection — because a
connection lost mid-transaction has discarded uncommitted work that cannot be
transparently resurrected.

## Decision

Give the Session an explicit transaction state: it is either *autocommit*
(unchanged) or *in an open transaction* started by `:begin`, closed by
`:commit`/`:rollback`. Queries in between run within it, and the one-live-result
guard (ADR 0005) composes unchanged — still one live stream at a time, now
inside the transaction.

Make reconnect depend on that state. In autocommit, reconnect stays silent. **In
an open transaction, a connection loss is not silently retried**: it surfaces as
a transaction-aborted error and drops the Session back to autocommit. The
guiding rule is that the console never silently re-runs or resurrects work the
user has explicitly bracketed.

## Consequences

- A query error inside a transaction poisons it (Memgraph rejects further
  statements until rollback); the console surfaces "transaction failed,
  `:rollback` to recover" and sends Bolt `RESET`/`ROLLBACK`, reusing the slice
  14 error taxonomy and RESET recovery.
- `:source` runs its statements in the *current* transaction, so a script can be
  a transactional batch. `:watch` is refused while a transaction is open — a
  repeating timer holding a transaction open is a footgun.
- Read-only mode (a separate decision) layers on as the Bolt access mode applied
  to transactions, implicit and explicit alike.
- The reconnect behaviour now has two paths a future reader must keep distinct;
  this ADR records why the transactional path deliberately does *not* reconnect.
