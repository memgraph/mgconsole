# 19. Runtime switch between the two interactive Frontends over a preserved Session

Date: 2026-06-11

## Status

Accepted

Amends ADR 0010: the choice of interactive Frontend is no longer permanent
for the life of a run.

## Context

ADR 0010 made the interactive Frontend a mutually exclusive choice "made at
startup" — pick the REPL (`--plain` or an incapable terminal) or the Workbench,
and live with it until you quit. The only way to change your mind was to quit
and relaunch.

Relaunching is not free: it drops the live Session. You lose the connection, the
`:param` store, the active Database, Read-only mode, and — most painfully — any
**open Transaction**, which ADR 0011 says the console will *never* silently
resurrect. So "just relaunch with `--plain`" throws away exactly the state a
working session has accumulated.

The seam for switching without that loss already exists. The Session is
Frontend-neutral (ADR 0002): the editor — the Buffer editor in the Workbench,
the input line in the REPL — is just text over a connection that knows nothing
about how a human drives it. Terminal ownership is already RAII: the Workbench's
`TerminalGuard` (alt-screen + raw mode) is scoped to `workbench::run`, so it
restores the terminal the moment that function returns. So a switch is not a
rewrite — it is tearing down one Frontend's terminal mode and standing up the
other's around a Session that stays live underneath.

One gap blocked this. CONTEXT.md's **Session** is defined to own the connection,
profile, Read-only mode, open Transaction, `:param` store, Schema, *and*
Settings. The code disagreed: the `Session` struct owned the connection,
transaction, Read-only mode, and active Database, but the `:param` store and
Settings lived as Frontend-local state inside `repl::run_loop` and
`WorkbenchState`. That gap never mattered while a Session never moved between
Frontends. A runtime switch makes it the central problem.

## Decision

- **A dispatch loop above the Frontends owns the Session.** Each interactive
  Frontend's run-function returns an outcome — `Quit` or `SwitchTo(Frontend)` —
  instead of `Result<()>`. The loop re-enters the other Frontend with the same
  Session, or exits. The Workbench's `TerminalGuard` dropping on return restores
  the terminal before the REPL takes over the normal screen; re-entering the
  Workbench calls `TerminalGuard::enter()` afresh. Each invocation is a clean
  scope, so no new terminal plumbing is needed.

- **The handed-across unit is the glossary's Session, made honest in code.** The
  `:param` store and Settings are hoisted out of the two Frontends into the
  Session-state bundle the loop owns and lends to each Frontend. The switch is
  then structurally "hand the one Session to the other Frontend" — nothing to
  thread by hand, and no new bug surface each time a Frontend gains state. The
  Schema, already Session-derived, rides along, so the arriving Frontend does not
  refetch. This closes the CONTEXT.md/code gap rather than widening it.

- **Triggered by the target-named Meta-commands `:repl` and `:workbench`.** A
  Frontend switch has meaning in *both* interactive Frontends, which is the
  definition of a Meta-command — these are the first Meta-commands whose effect
  is to leave the Frontend they were typed in. Target-named (not a single
  `:toggle`) so they are idempotent: `:repl` always means "be in the REPL,"
  a gentle no-op if already there. Command-only in v1; a Workbench *gesture* for
  the easy downward flick is a deliberate later increment (the REPL has no
  gesture vocabulary, so the upward direction stays command-only by design).

- **The active query editor text carries across; Buffers do not.** On a
  down-switch the active Buffer's draft seeds the REPL's input line (the
  meaningful direction — upward the line is empty, since you just submitted
  `:workbench`). Inactive Buffers and per-Buffer Result history are **not**
  preserved across a switch in v1: a round-trip is lossy and rebuilds one fresh
  Buffer on return. This honours the glossary's definition of Buffers as
  Workbench-local and ephemeral. Non-destructive view-parking (suspending and
  reattaching the whole Workbench view) is a deferred increment, tracked
  separately.

- **Guard conditions reuse existing precedent.** `:workbench` is gated on
  terminal *capability* (`supports_tui`: a capable tty and the `tui` feature) —
  **not** on the `--plain` startup flag, which was a preference, not a life
  sentence; launching `--plain` on a capable terminal and upgrading with
  `:workbench` is supported. In a build without the `tui` feature the command
  does not exist. `:repl` is always available — the REPL is the universal floor.
  Either switch is **refused while a query is in flight** ("session busy — cancel
  first"), reusing ADR 0005's one-live-result guard rather than silently aborting
  the query: Quit aborts because you are leaving, but a switch keeps the session,
  so it must not throw live results away.

## Consequences

- ADR 0010's startup selection logic still stands — it decides which Frontend you
  *start* in. What changes is that the choice is no longer permanent: the
  dispatch loop can re-enter the other Frontend. A future reader seeing the
  Frontends return an outcome enum instead of `Result<()>` should read this ADR,
  not "restore" the simpler signature.
- The code's `Session` (or an `InteractiveSession` bundle around it) grows to own
  params and Settings, matching CONTEXT.md. The two Frontends stop owning that
  state as locals and borrow it from the bundle.
- The open Transaction survives the switch for free: the switch never drops the
  connection, so ADR 0011's "never silently resurrect" is not even engaged — the
  transaction was never lost.
- v1 round-trips are lossy (tabs and Result history discarded on a down-switch).
  This is an accepted v1 limitation, not the end state; view-parking is the
  follow-up.
  - **Update (2026-06-11):** view-parking shipped (frontend-switch issue 04). The
    dispatch loop now parks the whole Workbench view — every Buffer (editor text +
    per-Buffer Result history) and the active index — across a switch and restores
    it on re-entry, so a Workbench → REPL → Workbench round-trip is non-destructive.
    The view holds no session state, so reattaching it onto the live Session needs
    no per-field reconciliation. (The active-Database *marker* is not yet rebound on
    an up-switch — the Session exposes no current-database getter — a small separate
    follow-up; the transaction marker is rebound.)
