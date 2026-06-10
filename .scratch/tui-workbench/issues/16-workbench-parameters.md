# 16 — Parameters in the workbench

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Bring the query-parameter workflow into the workbench, reusing the REPL's
parameter store and server-side `:param` evaluation. `:param <name> <expr>`
evaluates the Cypher expression server-side (existing params in scope) and stores
the resulting Value; `:params` / `:params clear` manage the set; the parameters
are bound to every query the workbench runs. A toggleable params drawer lists the
current parameters (`$name = value`).

## Acceptance criteria

- [ ] `:param` / `:params` / `:params clear` behave as in the REPL, reusing the
      existing store and server-side evaluation, and bind to every query run.
- [ ] A toggleable drawer lists the current parameters, ordered, with string
      values quoted (as the REPL lists them).
- [ ] A bad `:param` expression is reported without losing the session.
- [ ] The parameter commands and binding are covered at the reducer seam (the
      `:param`-parsing helpers are already unit-tested and reused).

## Blocked by

- `.scratch/tui-workbench/issues/02-nonblocking-execution-spine.md`
