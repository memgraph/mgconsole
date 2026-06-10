# 5. Session enforces one live result at a time with a runtime guard

Date: 2026-06-10

## Status

Accepted

## Context

Since slice 21 the Session returns a lazily-pulled `RecordStream` that shares the
connection (`Arc<Mutex<Conn>>`). Bolt is sequential: a second `run()` issued
while the previous result is still open corrupts the protocol. The invariant
"one live result at a time" must hold, and we had two ways to guarantee it.

## Decision

Guard the invariant **at runtime** while keeping the shared connection, rather
than enforcing it at compile time with a borrowing result
(`run(&mut self) -> QueryResult<'_>`).

`run()` errors (`Error::ResultStillOpen`) if a result is still live, and
recovers from an *abandoned* result (its `RecordStream` dropped before being
drained) by sending Bolt `RESET` before the new query. A drained or `discard`ed
stream clears the guard cleanly.

## Consequences

- The cancellation seam ADR 0002 paid for stays open: an independent
  `Arc<Mutex<Conn>>` handle can send `RESET` for a future Ctrl-C, which a
  borrow-enforced result would have made impossible (the borrow is held by the
  result consumer).
- Misuse is caught with a clear error instead of silent protocol corruption.
- Cost: the invariant is runtime-checked, not compiler-guaranteed. The
  mechanism (distinguishing a *live* stream from an *abandoned* one — e.g. via
  the result token's strong count — and clearing the guard on drain / discard /
  drop) is implemented in slice 14, alongside the rest of the Session error
  taxonomy and reconnect.
