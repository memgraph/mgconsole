# 01 — Core Cypher lexer

Status: ready-for-agent

## Parent

`.scratch/cypher-highlighting/PRD.md`

## What to build

A pure Core lexer that turns Cypher query text into a stream of total, typed
`Token`s — the input-side dual of the `Value` model (CONTEXT.md, ADR 0008). Each
token carries its byte span; concatenating the spans reproduces the input
exactly. A token is recognised by **shape alone**, never by consulting the
database, and reuses the same string/comment/backtick/escape-aware lexical
discipline `parse.rs` already encodes.

Token kinds are shape-based:

- **Word** — a run of alphanumerics, `_`, and `.` (so `point.distance` and
  `n.name` are each one Word). The lexer does **not** decide keyword vs function
  vs identifier — that classification is the Frontend's job (the keyword/function
  tables live in `cli/keywords.rs`), so a Word is emitted unclassified.
- **Number** — integer / float / scientific (`42`, `3.14`, `1e10`).
- **String** — `'…'`, `"…"`, and backtick `` `…` `` literals, honouring `\`
  escapes and doubled backticks; a `;` or keyword **inside** is part of the one
  String token, not split out.
- **Comment** — `// …` and `/* … */`, each one Comment token.
- **Parameter** — `$name`.
- **Punct** — punctuation/operators not absorbed above.
- **Whitespace** — runs of whitespace (including newlines).

The lexer is **total from the outset** (ADR 0003 idiom): every kind is emitted
from the first cut, even kinds no consumer colours yet. It is pure (no database,
no IO) and unit-tested in isolation, exactly as the `QueryAssembler` (slice 15)
and the clause scanner (slice 27) are.

This slice only adds the lexer and its API. Consumers (highlighter, and the
`parse.rs`/`clause.rs` consolidation) come in later slices.

## Acceptance criteria

- [ ] The lexer emits typed, byte-spanned tokens whose spans tile the whole
      input with no gaps or overlaps (re-concatenation is lossless).
- [ ] `'…'` / `"…"` / `` `…` `` are single String tokens, honouring `\` escapes
      and doubled backticks; a `;` or keyword inside one is not surfaced.
- [ ] `// …` and `/* … */` are single Comment tokens.
- [ ] `$name` is a Parameter token; integers/floats/scientific are Number tokens.
- [ ] Words are emitted unclassified (the lexer never consults a keyword table).
- [ ] An unterminated string or comment at end of input yields a token running
      to end of input (so a mid-typed line can still be coloured).
- [ ] Pure and unit-tested in the style of `parse.rs` / `clause.rs`; no database.

## Blocked by

- None - can start immediately
