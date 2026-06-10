# 10 — `:source <file>` run a file in-session

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add `:source <file>` to feed a file's contents through the **normal query path**,
one logical query at a time, reusing the line/query parsing from slice 15 rather
than a parallel reader. Because it is the same path, a sourced file may contain
Meta-commands (`:param`, `:set`, …), so a file can be a session-setup script, not
just queries. Sourced queries run in the current Transaction state (so within an
open transaction they join it, per ADR 0011). It **stops on the first error**
(predictable over permissive — and in an open transaction a failed statement has
poisoned it anyway), and **echoes each query** as it runs so a long source is
followable. Honoured identically in the REPL and Workbench.

## Acceptance criteria

- [ ] `:source <file>` runs each logical query in order through the normal query
      path, echoing each before its result.
- [ ] Meta-commands inside the file are honoured (e.g. a file that sets `:param`
      then runs a query using it).
- [ ] Execution stops on the first error with a clear message naming the failed
      statement; already-run statements are not rolled back (unless inside an
      explicit transaction).
- [ ] Inside an open transaction, sourced statements join that transaction.
- [ ] A missing or unreadable file reports a clear error without ending the
      session.

## Blocked by

- `.scratch/console-ergonomics/issues/05-explicit-transactions-state.md`
