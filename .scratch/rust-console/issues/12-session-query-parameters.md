# 12 — Query parameters passed through Session

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Session support for running a query with a set of named query parameters, so a
query referencing `$name` resolves against supplied values. This is the Core
capability; the `:param` Frontend commands are slice 20. Integration tested
against a live container.

As the first slice to add integration tests beyond 02, also introduce the
**shared-container test harness** described in `core/tests/common/mod.rs`
(one container per test binary + an aggressive `reset()`, tests serial within
the file) and convert the existing `session.rs` tests to it — so the growing
integration suite does not spin up a heavyweight Memgraph container per test.

## Acceptance criteria

- [ ] The Session accepts a set of named parameters alongside a query
- [ ] A query referencing a parameter returns the expected result
- [ ] Each parameter Value type (scalar, list, map, temporal, etc.) round-trips into a query
- [ ] Running with no parameters still works
- [ ] Integration test asserts parameterised queries against a container

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
