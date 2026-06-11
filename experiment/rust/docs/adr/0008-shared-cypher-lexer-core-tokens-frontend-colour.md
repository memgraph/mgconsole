# 8. Cypher tokenization is a Core capability; colour is a Frontend policy

Date: 2026-06-10

## Status

Accepted

## Context

The interactive REPL highlights query text as it is typed, so a writer can read
a query's structure and catch a mistyped reserved word before sending it (a
misspelled keyword renders in the identifier colour instead of the keyword
colour, so it stands out by contrast). Highlighting must distinguish every
lexical category — keyword, function, string, number, comment, query parameter,
punctuation, identifier — which the existing word-boundary splitter in the
Frontend cannot do.

Crucially, highlighting must be string- and comment-aware: a keyword appearing
inside `'…'` or `// …` is not a keyword and must not colour as one. The Core
already knows where strings and comments are — twice. `parse.rs` walks a
`Single/Double/Backtick/LineComment/BlockComment` state machine to find the `;`
that terminates a query; `clause.rs` walks the *same* states to skip strings and
comments while detecting import-ordering clauses. The old Frontend highlighter
re-derived a third, naive version that got this wrong. A fourth hand-rolled copy
is where the subtle cases (`\` escapes, doubled backticks, block-comment close)
drift — and drift here means wrong colours, the exact false positive the feature
exists to avoid.

The keyword and function tables live in the Frontend (`cli/keywords.rs`), so
"is this word a keyword?" is already a Frontend judgement, not a Core one.

## Decision

- **The Core gains a lexer** that turns query text into a stream of typed
  [Tokens](../../CONTEXT.md) — string, comment, word, number, query parameter,
  punctuation, whitespace — each with its byte span, reusing one string- and
  comment-aware state machine. A Token is recognised by shape alone, never by
  consulting the database; it is the input-side dual of the Core `Value`
  (ADR 0003), which is database output.
- **The Frontend owns colour.** The REPL highlighter consumes the Core Token
  stream and maps each Token — doing its own keyword/function table lookup — to
  an ANSI colour. Colour is presentation and stays Frontend-side, honouring the
  seam (ADR 0002): the Core yields lexical structure, never display.
- **`parse.rs` and `clause.rs` are not consolidated onto the lexer now.** They
  are tested and working; rewiring them is a separate architecture-deepening
  task with its own regression risk on the import and parser paths, deliberately
  not bundled with a display feature.

## Consequences

- The highlighter inherits escape/backtick/comment correctness from a shared,
  tested state machine instead of re-deriving it; keywords inside strings and
  comments stop colouring, eliminating that class of false positive.
- The Core now has a public lexical-token concept. This is hard to reverse once
  exposed, which is why it is recorded here rather than grown ad hoc.
- A known, accepted duplication remains: `parse.rs` and `clause.rs` keep their
  own string/comment scanning until a follow-up consolidates all three onto the
  lexer, behind their existing tests. The lexer is built so that consolidation
  is additive, not a rewrite.
- Colour policy and the vocabulary tables stay together in the Frontend; a
  future schema-aware source (labels, property keys) plugs in there, not in the
  Core lexer.
