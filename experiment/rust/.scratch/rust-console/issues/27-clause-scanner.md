# 27 — Clause scanner (vertices-first detection)

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

A fast lexer/scanner (e.g. via `logos`) that inspects query text and reports the
clauses relevant to import ordering — create, match, merge, index create/drop,
detach delete, remove, storage mode. It is a pure function from text to detected
clauses, used by parser mode (28) and to enforce vertices-first ordering in
parallel import (31). No database. Port the intent of `mgconsole`'s
clause-detection state machine, kept fast and DRY.

## Acceptance criteria

- [x] The scanner detects each relevant clause from query text
- [x] Detection is robust to casing, comments, and multiline/multi-query input
- [x] Results compose across the lines of one logical query
- [x] The scanner is a pure function with no DB/session dependency
- [x] Comprehensive pure unit tests, including ordering-relevant edge cases

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
