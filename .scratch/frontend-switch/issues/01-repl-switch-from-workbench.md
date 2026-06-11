# Down-switch: `:repl` from the Workbench, over a preserved Session

Status: done

## What to build

The structural tracer bullet for runtime Frontend switching (ADR 0019). Stand up
a dispatch loop above the two interactive Frontends that owns the Session-state
bundle and re-enters whichever Frontend is chosen, and wire the first direction:
typing `:repl` in the Workbench drops you into the REPL with the **same live
Session** underneath.

Three things move together because none is demoable alone:

- **Session-state bundle.** Hoist the `:param` store and Settings out of
  `repl::run_loop` and `WorkbenchState` into the bundle the dispatch loop owns
  and lends `&mut` to each Frontend. This matches CONTEXT.md's Session definition
  (the connection, profile, Read-only mode, open Transaction, active Database,
  params, Schema, and Settings are all Session-scoped). The Schema rides along,
  so the REPL does not refetch it.
- **Outcome enum + dispatch loop.** Each interactive Frontend's run-function
  returns `Quit | SwitchTo(Frontend)` instead of `Result<()>`. The loop exits on
  `Quit` or re-enters the other Frontend on `SwitchTo`. The Workbench's
  `TerminalGuard` dropping as `run` returns restores the terminal before the REPL
  takes the normal screen — no new terminal teardown code.
- **`:repl` as a Meta-command in the Workbench.** It is a genuine cross-frontend
  Meta-command (recognised in both Frontends), typed at the Workbench Command
  line. Idempotent: in the REPL it is a gentle no-op ("already in the REPL").
  Appears in the Workbench `:help` / command vocabulary.

The REPL input line arrives **empty** on this slice — carrying the active Buffer
draft across is issue 03. Inactive Buffers and Result history are discarded
(lossy round-trip, per ADR 0019).

**Guard — refuse while busy.** `:repl` is refused while a query is in flight,
with the one-live-result message family (`"session busy — cancel first"`, ADR
0005), rather than aborting the query — a switch keeps the session, so it must
not throw live results away. This guard is Workbench-only: the REPL `block_on`s
each query to completion, so it never has an in-flight query at its prompt.

## Acceptance criteria

- [ ] The `:param` store and Settings are owned by the dispatch-loop Session
      bundle, not by the individual Frontends; both Frontends read/write them
      through the bundle.
- [ ] Each interactive Frontend's run-function returns `Quit | SwitchTo(Frontend)`;
      the dispatch loop exits on `Quit` and re-enters the other Frontend on
      `SwitchTo`.
- [ ] Starting in the Workbench (default), typing `:repl` lands in the REPL with
      the connection, open Transaction, params, Settings, active Database, and
      Read-only mode all intact and live.
- [ ] An open Transaction begun in the Workbench is still open in the REPL after
      the switch (not silently committed, rolled back, or resurrected).
- [ ] `:repl` typed in the REPL is a no-op with an "already in the REPL" message.
- [ ] `:repl` is refused while a query is in flight in the Workbench, with the
      one-live-result "cancel first" message; the running query is not aborted.
- [ ] `:repl` appears in the Workbench `:help` / command vocabulary.
- [ ] The terminal is left clean after the switch (alt-screen and raw mode
      restored before the REPL prompt appears).

## Blocked by

- None - can start immediately
