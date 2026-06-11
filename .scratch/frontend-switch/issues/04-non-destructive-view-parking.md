# Non-destructive Frontend switch: park and restore the Workbench view

Status: needs-triage

_Blocked on the v1 runtime Frontend switch landing (ADR 0019). Don't grab until
`:repl`/`:workbench` exist._

## Context

ADR 0019 ships runtime switching between the REPL and the Workbench over a
preserved Session. v1 is deliberately **lossy on a round-trip**: a down-switch
(Workbench → REPL) keeps only the active Buffer's editor text; all inactive
Buffers and every Buffer's Result history are discarded, and an up-switch builds
one fresh Buffer. So flicking down to the REPL and back up loses your tabs.

That's an accepted v1 limitation, not the end state. This issue is the follow-up
to make round-trips non-destructive.

## What we want

Switching Workbench → REPL → Workbench should restore the Workbench view you
left: all Buffers (editor text + per-Buffer Result history) and the active Buffer
index, exactly as they were.

## Approach sketch (from the ADR discussion)

- The dispatch loop holds the suspended Workbench **view** — the `buffers` set,
  per-Buffer Result history, and `active` index — **alongside** the Session-state
  bundle, *not* inside it. Buffers stay Frontend-local; the Session bundle stays
  Frontend-neutral. Model it as the loop carrying an `Option<WorkbenchView>` that
  the Workbench detaches on `SwitchTo(Repl)` and reattaches on re-entry.
- On re-entry, **rebind the parked view to the live Session**, don't trust a
  snapshot. While you were down in the REPL you may have changed the active
  Database, mutated params/Settings, or opened/closed a Transaction — the parked
  Buffers must reattach to the *current* Session state, not the one captured at
  detach time. Reconciliation is the real work here, and the reason it was cut
  from v1.
- Decide what happens to the REPL line edited during the detour: does it seed /
  overwrite the active Buffer's editor text on return, or is it dropped? (Lean:
  if the editor text was carried down and edited, carry the edit back up into the
  same Buffer.)
- Queries run in the REPL during the detour ran against the same Session but are
  not Buffer-scoped, so they legitimately won't appear in any Buffer's Result
  history. That's expected, not a bug — note it so a reviewer doesn't "fix" it.

## Out of scope

- The Workbench gesture (key chord) for the downward flick — separate deferred
  increment from ADR 0019 (v1 is command-only).

## Blocked by

- Issue 01 (down-switch + dispatch loop + Session bundle)
- Issue 02 (up-switch — round-trips must exist before they can be made
  non-destructive)
- Issue 03 (editor-text carry — view-parking extends the same handoff)
