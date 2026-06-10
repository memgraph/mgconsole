# 11 — Static keyword/function completion popup

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Bring completion online in the workbench editor. Wire the existing `Completer`
(its static keyword/function vocabulary) and `word_start` to a completion popup:
on trigger, offer candidates for the word under the cursor and insert the chosen
one. This is the completion UI the live schema source (slice 12) later plugs
into as a second source — no change to the cursor handling when it does.

## Acceptance criteria

- [ ] Triggering completion offers keyword/function candidates for the prefix
      under the cursor, matched case-insensitively, and inserts the selection.
- [ ] The popup navigates with the keyboard and dismisses cleanly; an empty
      prefix offers nothing (as the REPL completer already decides).
- [ ] The existing `Completer`/`word_start` are reused unchanged.
- [ ] Completion state (open, candidates, selection, insertion) is covered at the
      reducer seam.

## Blocked by

- `.scratch/tui-workbench/issues/01-frontend-selection-shell-lifecycle.md`
