# 02 — Non-blocking query execution spine

Status: done

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

The execution edge of the workbench (ADR 0010). Pressing Enter submits the whole
editor buffer; the existing query assembler splits it on `;` into one or more
statements, run **sequentially** on the single Session. Each query runs as a
**cancellable async task** driven by the tokio runtime, emitting query-lifecycle
events (started, record arrived, completed with summary, errored) back to the
reducer over a channel. The render loop multiplexes terminal-input events and
these channel events and never blocks.

This slice shows the result minimally (the final result and a status line with
row count + elapsed time); the navigable streaming table is slice 03. A query
error is surfaced in the status without ending the session (ADR 0005). While a
query is running, submitting another is refused by the reducer with a "session
busy — cancel first" status, honouring the one-live-result guard.

## Acceptance criteria

- [x] Enter submits the editor buffer; a multi-statement buffer is split and run
      in order on the one Session; a buffer with no `;` runs as a single query.
      (`submit` + `split_statements` via the Core `QueryAssembler`; reducer tests.)
- [x] A query runs as a cancellable task; lifecycle events flow to the reducer
      and the UI stays responsive (no frozen render loop) while it runs.
      (`run_query` spawned task streams events over a channel; the loop
      `tokio::select!`s terminal input against the channel and never blocks.)
- [x] The result's row count and elapsed time appear in the status line.
      (`on_completed` reuses the REPL's `format_summary`; test asserts the line.)
- [x] A query error is reported without losing the session; the next query runs.
      (`on_failed` surfaces the error, stays idle, next submit runs; test.)
- [x] Submitting a second query while one is in flight is rejected with a
      "session busy" status (one-live-result guard). (`submit` busy guard; test.)
- [x] The submit/lifecycle handling is covered at the reducer seam with a faked
      execution edge (no terminal, no database), like `run_loop`'s `QueryRunner`.
      (`workbench::update` tests feed hand-built lifecycle events.)

## Blocked by

- `.scratch/tui-workbench/issues/01-frontend-selection-shell-lifecycle.md`
