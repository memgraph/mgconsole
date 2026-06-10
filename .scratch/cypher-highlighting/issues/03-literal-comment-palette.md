# 03 — Literal & comment palette: string, number, comment

Status: done

## Parent

`.scratch/cypher-highlighting/PRD.md`

## What to build

Extend the highlighter's colour mapping to the shape-based literal and comment
categories the lexer already emits:

- String → green
- Number → magenta
- Comment → grey (bright-black)

Punctuation and identifier words stay terminal-default — deliberately uncoloured,
so the coloured categories pop and a typo'd keyword (which renders as a plain
identifier) stands out by contrast.

Because an unterminated string or comment lexes as a token running to end of
input (slice 01), a half-typed `RETURN 'foo` visibly colours green up to the
cursor until the quote is closed — a bonus mid-typing signal, kept on purpose.

Query parameters (`$x`) are coloured separately in slice 04.

## Acceptance criteria

- [ ] String tokens render green, Number tokens magenta, Comment tokens grey.
- [ ] Punctuation and identifier words remain uncoloured (terminal default).
- [ ] An unterminated string while typing colours green to end of buffer.
- [ ] Colour assignments are covered by `highlight()` unit tests.

## Blocked by

- `.scratch/cypher-highlighting/issues/02-highlighter-cutover-keyword-function.md`
