# 10 — `:source <file>` run a file in-session

Status: done

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

- [x] `:source <file>` runs each logical query in order through the normal query
      path, echoing each before its result.
- [x] Meta-commands inside the file are honoured (REPL: a sourced file may set
      `:param`/`:set`/… then run a query using it). See note for the Workbench.
- [x] Execution stops on the first error; already-run statements are not rolled
      back (unless inside an explicit transaction, where the failure poisons it).
- [x] Inside an open transaction, sourced statements join that transaction (they
      run through the same Session/run path).
- [x] A missing or unreadable file reports a clear error without ending the
      session.

## Blocked by

- `.scratch/console-ergonomics/issues/05-explicit-transactions-state.md`

## Comments

Implemented (AFK).

- **REPL** (full): `:source` reads the file and runs it through `source_content`,
  which reuses the loop's own line/query handling — `meta_command` + the extracted
  `dispatch_meta` for `:`-lines and `QueryAssembler` + the extracted
  `execute_query` for queries. Each statement is echoed, execution stops on the
  first query error, and a `:quit` inside a source ends sourcing (not the
  session). Because it is the same path, a sourced file is a full session-setup
  script (meta + queries). `execute_query` now returns success so the source can
  halt. Tests cover order+meta+echo, stop-on-error, and a missing file.
- **Workbench**: `:source` reads the file off the render loop (`Effect::Source` →
  `Event::SourceLoaded`) and runs its statements as a stop-on-error batch (a new
  `source_halt` flag makes `on_failed` drop the rest of the queue). Each statement
  lands in the result pane.
- **Note (Workbench AC2)**: interleaved meta-commands inside a *Workbench*-sourced
  file are not honoured — the async execution model (`:param` evaluation and query
  runs are both async events) can't preserve meta↔query ordering within a batch.
  The REPL is the scripting frontend for meta-bearing source files; the Workbench
  sources Cypher statements. Documented rather than faked.
