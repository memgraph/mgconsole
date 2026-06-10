# 28 — Parser-only mode

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The parser Import mode: read queries from the input (slice 25's pipe path) and
report on them using the clause scanner (27) without executing anything against
the database. Useful for validating an import file before touching Memgraph.
Honour the parser-stats flags (collect/print per-query statistics).

## Acceptance criteria

- [ ] Parser mode reads the input and reports per the scanner without any DB execution
- [ ] No queries are sent to Memgraph in this mode
- [ ] Parser statistics are collected and printed per the relevant flags
- [ ] An end-to-end test runs parser mode over a fixture and asserts the report, confirming no DB side effects

## Blocked by

- `.scratch/rust-console/issues/25-serial-import-pipe.md`
- `.scratch/rust-console/issues/27-clause-scanner.md`
