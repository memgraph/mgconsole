# 04 — Read-only mode (Bolt READ access mode)

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add Read-only mode (`CONTEXT.md`): a session-wide safety guard that sets every
Transaction's Bolt access mode to READ, so **Memgraph itself rejects writes** —
the Clause scanner is never asked to police it. It is settable as a `--read-only`
CLI flag and via `:set readonly on`, and (once profiles exist, issue 03)
storable in a connection profile. The asymmetry is deliberate: it can be turned
**on** at runtime, but turned **off** only at connect time or via a profile —
`:set readonly off` at runtime is refused. A `[read-only]` marker shows in the
REPL prompt and the Workbench status bar whenever it is active.

## Acceptance criteria

- [ ] With read-only active, a writing query is rejected by the server (Bolt READ
      access mode applied to the transaction), verified against a live database.
- [ ] `--read-only` and `:set readonly on` both activate it; the Clause scanner
      is not involved in the decision.
- [ ] `:set readonly off` at runtime is refused with a clear message explaining
      it can only be cleared at connect time or via a profile.
- [ ] A `[read-only]` marker appears in the REPL prompt and Workbench status bar
      while active, and is absent otherwise.
- [ ] Read-only applies to both autocommit and explicit transactions (a `:begin`
      cannot upgrade a read-only session to write — see issue 05).

## Blocked by

- `.scratch/console-ergonomics/issues/01-set-mechanism-vertical-display.md`
