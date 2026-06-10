# 09 — Undoable buffer replacement + `Ctrl+Z`/`Ctrl+Y`

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics/PRD.md`

## What to build

Make the operations that replace the whole editor buffer — auto-format, `:load`,
and history recall — undoable, and wire undo/redo. Today these rebuild the editor
widget (a fresh text area), discarding its undo stack, so a format can't be
reverted and recalling over unsaved text loses it irrecoverably. Replace the
buffer through the editor's own edit operations (select-all + insert) so the
change joins the undo history, and bind `Ctrl+Z` (undo) / `Ctrl+Y` (redo).

## Acceptance criteria

- [ ] After auto-format / `:load` / history recall, `Ctrl+Z` restores the previous
      buffer text.
- [ ] `Ctrl+Y` redoes; ordinary typed edits remain undoable as before.
- [ ] No buffer-replacing operation silently discards the undo history.
- [ ] Reducer tests cover undo after a format/replace and redo after an undo.

## Blocked by

None - can start immediately.
