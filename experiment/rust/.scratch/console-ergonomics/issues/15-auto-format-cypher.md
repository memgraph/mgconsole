# 15 — Auto-format Cypher (Workbench editor gesture)

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add a Workbench gesture (a key chord) that pretty-prints the Cypher in the editor
buffer, using the Core lexer's Tokens (`CONTEXT.md`) for clause/keyword breaks
rather than a new parser. Formatting is a pure function from query text to query
text over the Token stream — testable without a terminal — so the same formatter
could later back a `:format` command if wanted. It is openly Workbench-only (a
gesture, not a cross-Frontend Meta-command).

## Acceptance criteria

- [ ] A key chord reformats the editor's Cypher (clause newlines, consistent
      spacing/keyword case) driven by the Core lexer Token stream.
- [ ] Formatting is a pure `text -> text` function with unit tests over
      representative queries (multi-clause, nested, comments preserved).
- [ ] Formatting never changes query semantics (round-trips through the lexer to
      the same Token sequence modulo whitespace).
- [ ] Invalid/partial input is left untouched with a clear status, not mangled.

## Blocked by

None - can start immediately.
