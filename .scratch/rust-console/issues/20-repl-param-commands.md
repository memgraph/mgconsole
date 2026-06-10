# 20 — `:param` / `:params` / `:params clear`

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The interactive parameter commands. `:param <name> <expression>` evaluates the
Cypher expression server-side (with existing parameters in scope), stores a copy
of the resulting Value, and makes it available as `$name` in subsequent queries.
`:params` lists the current parameters; `:params clear` removes them all. Uses
the Session parameter passthrough from slice 12. The command-line parsing of
`:param`/`:params` is a pure unit-tested function.

## Acceptance criteria

- [ ] `:param name expr` evaluates the expression server-side and stores the result
- [ ] A stored parameter is usable as `$name` in a later query
- [ ] Existing parameters are in scope when evaluating a new parameter expression
- [ ] `:params` lists current parameters; `:params clear` empties them
- [ ] A malformed `:param`/`:params` command reports a clear error without ending the session
- [ ] Command parsing is covered by pure unit tests

## Blocked by

- `.scratch/rust-console/issues/16-repl-execute-loop.md`
- `.scratch/rust-console/issues/12-session-query-parameters.md`
