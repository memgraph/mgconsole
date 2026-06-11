# Cypher syntax highlighting in the REPL

## Why

Highlighting query text as it is typed is a **signal**, not decoration: it lets
a writer read a query's structure and catch a mistyped reserved word before
sending it. A misspelled keyword renders in the identifier colour instead of the
keyword colour, so it stands out by contrast. The signal can only ever cover the
closed vocabulary (Cypher/Memgraph keywords and built-in functions) — it cannot
catch typos in labels, properties, or variables, which the console does not know
without the database.

Highlighting already existed (slice 18) but was naive: a word-boundary splitter
that coloured only keywords/functions, was **not** string/comment-aware (so a
keyword inside `'…'` lit up), and was off by default. This feature replaces its
internals with a proper lexer, widens the palette, and turns it on by default.

## Design (settled)

See the decisions of record:

- **ADR 0008** — Cypher tokenization is a Core capability; colour is a Frontend
  policy. A Core lexer emits typed [`Token`s](../../CONTEXT.md); the Frontend
  maps token + keyword/function lookup to colour. `parse.rs`/`clause.rs` are
  consolidated onto the lexer as a follow-up, behind their existing tests.
- **ADR 0009** — interactive input colouring is on by default, via
  `--color=auto|always|never`, honouring `NO_COLOR`.
- **CONTEXT.md** — the `Token` glossary term (the input-side dual of `Value`).

### Palette (16-colour ANSI, theme-mapped, not truecolor)

| Keyword | Function | String | Number | Comment | Parameter | Operator/punct | Identifier |
|---|---|---|---|---|---|---|---|
| yellow | cyan | green | magenta | grey | blue | default | default |

Booleans/`null` stay keywords; labels are not separately coloured; punctuation
and identifiers stay terminal-default so the coloured categories pop and a
typo'd keyword stands out by contrast.

### Tokenization riders

- `.` is a word constituent, so dotted built-ins (`point.distance`) still match
  the function table and `n.name` is one identifier token.
- Function detection is table-based; user procedures (`apoc.foo`) render plain.
- Unterminated strings/comments colour to end of buffer — a bonus mid-typing
  signal. Multiline is free: rustyline 15 hands `highlight()` the whole buffer.

## Slices

1. Core Cypher lexer
2. Highlighter cutover (keyword + function, string/comment-correct)
3. Literal & comment palette (string, number, comment)
4. Query-parameter colour (`$x`)
5. `--color` flag, on by default
6. Consolidate `parse.rs` onto the lexer
7. Consolidate `clause.rs` onto the lexer
