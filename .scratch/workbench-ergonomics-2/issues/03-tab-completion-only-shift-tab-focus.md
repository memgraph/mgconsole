# 03 — Editor `Tab` = completion-only; `Shift+Tab` = focus-switch

Status: done

## Parent

`.scratch/workbench-ergonomics-2/PRD.md` — implements **ADR 0017** (the
`Tab`/`Shift+Tab` keyspace).

## What to build

Remove `Tab`'s overload in the Buffer editor — the original complaint that "tab to
move panel, and tab for auto complete is confusing".

- **`Tab` means completion only**: it opens the completion popup and cycles to the
  next candidate. It no longer cycles pane focus when there is nothing to complete
  (with an empty prefix / no candidates it is simply a no-op).
- **`Shift+Tab`** cycles to the **previous candidate** while the completion popup
  is open, and **switches focus** between the editor and the results pane when the
  popup is closed. `Shift+Tab` (BackTab / `CSI Z`) is delivered reliably on every
  terminal, unlike `Ctrl+Tab`.

Independent of the command line (issue 01), but together they are ADR 0017.

## Acceptance criteria

- [ ] In the editor, `Tab` opens/cycles completion and never switches pane focus;
      with no candidates it is a no-op.
- [ ] `Shift+Tab` cycles to the previous candidate when the popup is open.
- [ ] `Shift+Tab` switches focus editor↔results when the popup is closed (and from
      the results pane returns focus to the editor).
- [ ] Reducer tests cover Tab-no-longer-switches-focus, Shift+Tab prev-candidate,
      and Shift+Tab focus-switch in both directions.

## Blocked by

None - can start immediately.
