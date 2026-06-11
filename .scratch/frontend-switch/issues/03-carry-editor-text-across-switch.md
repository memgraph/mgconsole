# Carry the active query editor text across the switch

Status: ready-for-agent

## What to build

The "query editor is connected" headline (ADR 0019): the editor text follows you
across a Frontend switch, so a draft you were composing isn't lost.

- **Down (Workbench → REPL):** the active Buffer's editor text seeds the REPL's
  initial input line. This is the meaningful direction — a half-drafted Cypher
  query in the Buffer arrives on the REPL prompt ready to edit or run.
- **Up (REPL → Workbench):** the REPL's current input line becomes the new
  Buffer's editor text. In practice this is usually empty (you just submitted
  `:workbench`), so it is a near-no-op included for symmetry; carry it anyway so
  the behavior is consistent rather than special-cased.

Only the **active** editor text travels. Inactive Buffers and per-Buffer Result
history are still discarded (lossy round-trip, per ADR 0019 — non-destructive
preservation is issue 04).

## Acceptance criteria

- [ ] Drafting a query in the active Workbench Buffer (without submitting it),
      then `:repl`, leaves that text sitting on the REPL input line.
- [ ] A multi-line draft survives the down-switch intact (the REPL assembles
      multi-line input, so the carried text is usable).
- [ ] A non-empty REPL input line carried up becomes the new Buffer's editor
      text; an empty line yields an empty Buffer (no spurious content).
- [ ] The carried text is editable/runnable in the destination Frontend like any
      other input (not a frozen or read-only seed).

## Blocked by

- Issue 01 (the down-switch, which is the meaningful carry direction).
- Issue 02 (the up-switch, for the symmetric carry).
