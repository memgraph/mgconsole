# 15 — Line/query parsing: multiline + multi-query-per-line + carryover

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The pure parsing logic that turns raw input lines into complete queries: a query
spans multiple lines until a terminating semicolon; several queries may appear
on one line; and any unfinished trailing fragment carries over to the next
input. Quotes and escapes are respected so a semicolon inside a string literal
does not terminate a query. Port the behaviour of `mgconsole`'s line parsing.
Pure function, no database, heavily unit-tested.

## Acceptance criteria

- [x] A query spanning multiple lines is assembled and completes on its terminating semicolon
- [x] Multiple queries on one line are split into separate queries
- [x] A trailing unfinished fragment is carried over to the next input
- [x] Semicolons and terminators inside string literals / comments do not split or terminate
- [x] Comprehensive pure unit tests cover these cases

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
