# 16 — Parameters in the workbench

Status: done

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

- [x] `:param` / `:params` / `:params clear` behave as in the REPL, reusing the
      existing store and server-side evaluation, and bind to every query run.
      (`handle_meta` reuses `meta_command`; `Effect::EvaluateParam` runs
      `RETURN <expr>`; `ParamEvaluated` stores into `state.params`, which
      `start_query` binds to every `RunQuery`.)
- [x] A toggleable drawer lists the current parameters, ordered, with string
      values quoted (as the REPL lists them). (Ctrl-P / `:params` open the drawer;
      `draw_params` reuses `repl::format_params`.)
- [x] A bad `:param` expression is reported without losing the session.
      (`ParamEvaluated(Err)` → status error; nothing stored; next query runs.)
- [x] The parameter commands and binding are covered at the reducer seam (the
      `:param`-parsing helpers are already unit-tested and reused). (set/evaluate,
      bind-to-next, bad-expression, list/clear, and Ctrl-P toggle tests.)

## Blocked by

- `.scratch/tui-workbench/issues/02-nonblocking-execution-spine.md`
