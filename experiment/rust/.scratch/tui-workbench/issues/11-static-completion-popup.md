# 11 — Static keyword/function completion popup

Status: done

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Bring completion online in the workbench editor. Wire the existing `Completer`
(its static keyword/function vocabulary) and `word_start` to a completion popup:
on trigger, offer candidates for the word under the cursor and insert the chosen
one. This is the completion UI the live schema source (slice 12) later plugs
into as a second source — no change to the cursor handling when it does.

## Acceptance criteria

- [x] Triggering completion offers keyword/function candidates for the prefix
      under the cursor, matched case-insensitively, and inserts the selection.
      (Tab → `open_completion` via the state's `Completer`; `apply_completion`
      replaces the prefix with the selected candidate.)
- [x] The popup navigates with the keyboard and dismisses cleanly; an empty
      prefix offers nothing (as the REPL completer already decides). (Up/Down/Tab
      cycle with wrap, Esc dismisses; an empty prefix yields no candidates, so the
      popup never opens and Tab falls back to focus-switch.)
- [x] The existing `Completer`/`word_start` are reused unchanged.
      (`WorkbenchState::completer = Completer::with_static_vocabulary()`;
      `EditorState::word_under_cursor` uses `syntax::word_start`.)
- [x] Completion state (open, candidates, selection, insertion) is covered at the
      reducer seam. (open, insert, cycle-wrap, Esc-dismiss, empty-prefix tests.)

## Blocked by

- `.scratch/tui-workbench/issues/01-frontend-selection-shell-lifecycle.md`
