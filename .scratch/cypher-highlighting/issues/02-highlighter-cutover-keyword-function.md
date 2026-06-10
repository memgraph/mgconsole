# 02 — Highlighter cutover: keyword + function on the lexer

Status: done

## Parent

`.scratch/cypher-highlighting/PRD.md`

## What to build

Replace the internals of the Frontend's `syntax::highlight()` so it consumes the
Core lexer's `Token` stream (slice 01) instead of its own word-boundary
splitter. The highlighter classifies **Word** tokens against the existing
keyword/function tables (`cli/keywords.rs`) and colours:

- keyword → yellow
- function → cyan (**changed** from today's bright-red)

with keyword winning when a name is in both tables (as today), matched
case-insensitively. All other tokens pass through uncoloured for now (later
slices widen the palette).

The visible payoff of moving onto real tokens is the **false-positive fix**: a
keyword or function name appearing inside a string or comment is now a String /
Comment token, not a Word, so it is no longer coloured. `highlight()` stays a
pure function over text, testable without a terminal. The completer and
`word_start` machinery are untouched.

This is a Frontend colour-policy change only; per ADR 0008 the colour mapping and
the vocabulary tables stay Frontend-side.

## Acceptance criteria

- [ ] `highlight()` is driven by the Core lexer's tokens, not the old splitter.
- [ ] Keywords colour yellow and functions colour cyan, case-insensitively, with
      keyword winning a name present in both tables.
- [ ] A keyword or function name inside a string literal or a comment is **not**
      coloured.
- [ ] Functions render cyan, not bright-red.
- [ ] `highlight()` remains a pure `&str`-in / text-out function with unit tests;
      the completer and `word_start` behaviour and their tests are unchanged.

## Blocked by

- `.scratch/cypher-highlighting/issues/01-core-cypher-lexer.md`
