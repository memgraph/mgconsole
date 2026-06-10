# 05 — TUI editor input highlighting

Status: done

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

- [x] Typed Cypher is highlighted by token category using ratatui styles derived
      from the shared `HighlightCategory`. (`highlight::highlight_line` lexes the
      line, `categorize`s each token, and `style_for` maps it to a ratatui Style;
      the editor draw renders the styled spans, re-run every frame/keystroke.)
- [x] A keyword/function inside a string or comment is not coloured (inherited
      from the lexer, as in the REPL). (Via `categorize`; unit-tested here too.)
- [x] Highlighting respects the resolved colour setting (off → no styling).
      (`highlight_line(.., false)` yields a single unstyled span; `state.color`.)
- [x] The category→style mapping is covered by unit tests. (`style_for` +
      `highlight_line` tests, pure — no terminal.)

## Note

`tui-textarea` 0.7 has no per-token styling hook (only cursor/selection/search),
so the workbench renders the editor lines itself with highlighted spans and uses
the widget purely as the edit/cursor model (`lines()`/`cursor()`). The terminal
cursor is placed via `Frame::set_cursor_position`; a simple cursor-following
scroll keeps a long query editable.

## Blocked by

- `.scratch/tui-workbench/issues/01-frontend-selection-shell-lifecycle.md`
- `.scratch/tui-workbench/issues/04-highlight-category-refactor.md`
