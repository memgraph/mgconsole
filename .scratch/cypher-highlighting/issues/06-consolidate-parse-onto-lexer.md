# 06 — Consolidate `parse.rs` onto the Core lexer

Status: ready-for-agent

## Parent

`.scratch/cypher-highlighting/PRD.md`

## What to build

The deferred deepening from ADR 0008: rewire `core/src/parse.rs` so the
`QueryAssembler`'s statement splitting sits on the Core lexer's token stream
(slice 01) — splitting queries on `;` Punct tokens that fall outside strings and
comments — instead of carrying its own duplicate `Single/Double/Backtick/
LineComment/BlockComment` state machine. One lexical state machine, not two.

This is a pure internal refactor behind the existing `parse.rs` tests; the
`QueryAssembler` public surface and behaviour (multiline assembly, several
queries per line, `;` inside strings/comments, unterminated-tail carry-over) do
not change.

## Acceptance criteria

- [ ] `parse.rs` derives its query boundaries from the Core lexer; its private
      string/comment state machine is removed.
- [ ] All existing `parse.rs` tests pass unchanged.
- [ ] `QueryAssembler`'s public signatures and observable behaviour are unchanged.

## Blocked by

- `.scratch/cypher-highlighting/issues/01-core-cypher-lexer.md`
