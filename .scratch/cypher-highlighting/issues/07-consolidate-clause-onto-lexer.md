# 07 — Consolidate `clause.rs` onto the Core lexer

Status: ready-for-agent

## Parent

`.scratch/cypher-highlighting/PRD.md`

## What to build

The second half of the ADR 0008 deepening: rewire `core/src/clause.rs` so
`scan_clauses` consumes the Core lexer's **Word** tokens (lowercased) instead of
its own `keyword_tokens` scan, which re-implements the same string/comment
skipping a third time. After this slice the lexical state machine lives in
exactly one place.

A pure internal refactor behind the existing `clause.rs` tests; clause detection
behaviour is unchanged — keywords inside strings/comments are still ignored (now
because they are String/Comment tokens, not Words), and word boundaries still
keep `created` from matching `create`.

## Acceptance criteria

- [ ] `clause.rs` derives its words from the Core lexer's Word tokens; its
      private `keyword_tokens` string/comment scan is removed.
- [ ] All existing `clause.rs` tests pass unchanged.
- [ ] `scan_clauses`'s observable behaviour is unchanged.

## Blocked by

- `.scratch/cypher-highlighting/issues/01-core-cypher-lexer.md`
