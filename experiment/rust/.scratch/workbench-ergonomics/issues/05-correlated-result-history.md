# 05 — Correlated Result history: failures are navigable entries

Status: done

## Parent

`.scratch/workbench-ergonomics/PRD.md`

## What to build

Make the Result history (`CONTEXT.md`) a complete, self-describing record. Every
submission becomes exactly one navigable entry pairing the originating query with
its outcome — a Query result (Records + Notifications) or, when the query failed,
the **error** in place of Records (plus any Notifications the server sent with the
failure). A failed query, which today leaves only a transient status that the next
keystroke overwrites, becomes a navigable entry carrying its query and error.

As the user navigates the Result history (`[`/`]`), the results pane shows the
**originating query** for the displayed entry (truncated in the pane header, full
via cell-expand) alongside the outcome; Notifications stay in their drawer. So a
failure is reviewable, not a message that scrolled away.

## Acceptance criteria

- [ ] Every submission produces exactly one Result-history entry; a failed query
      (including one that fails before any Record) is an entry carrying its
      originating query and the error.
- [ ] Navigating to a failed entry shows the query and the error rendered in the
      result area, not a blank table.
- [ ] The originating query is shown for the displayed entry (pane header) at
      every history position; cell-expand shows it in full.
- [ ] A successful entry still shows its Records and the Notifications drawer,
      unchanged.
- [ ] Reducer tests cover the failure-entry and the per-entry query correlation.

## Blocked by

None - can start immediately.
