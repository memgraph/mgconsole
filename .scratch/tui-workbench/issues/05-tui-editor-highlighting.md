# 05 — TUI editor input highlighting

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Colour the query as it is typed in the workbench editor, the same lexical signal
the REPL gives. Map the shared `HighlightCategory` (slice 04) to ratatui styles
and apply them per token to the editor buffer's rendered spans, re-highlighting
on each keystroke. The palette mirrors the REPL's meaning (keyword, function,
string, number, comment, parameter; identifiers/punctuation plain) so a misspelt
keyword renders plain and stands out by contrast. Honours the resolved colour
setting (a monochrome workbench shows no highlight).

## Acceptance criteria

- [ ] Typed Cypher is highlighted by token category using ratatui styles derived
      from the shared `HighlightCategory`.
- [ ] A keyword/function inside a string or comment is not coloured (inherited
      from the lexer, as in the REPL).
- [ ] Highlighting respects the resolved colour setting (off → no styling).
- [ ] The category→style mapping is covered by unit tests.

## Blocked by

- `.scratch/tui-workbench/issues/01-frontend-selection-shell-lifecycle.md`
- `.scratch/tui-workbench/issues/04-highlight-category-refactor.md`
