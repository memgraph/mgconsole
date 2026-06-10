# 04 — Query-parameter colour ($x)

Status: ready-for-agent

## Parent

`.scratch/cypher-highlighting/PRD.md`

## What to build

Colour **Parameter** tokens (`$name`) blue. These are the references resolved by
the `:param` family (slice 20), so giving them their own colour lets a writer see
at a glance which `$name`s in a query are bound parameters versus typos.

Only a real Parameter token colours — a literal `$` inside a string is part of a
String token and stays uncoloured, falling out of the lexer naturally.

## Acceptance criteria

- [ ] `$name` parameter references render blue.
- [ ] A `$` inside a string literal is not coloured as a parameter.
- [ ] The colour assignment is covered by `highlight()` unit tests.

## Blocked by

- `.scratch/cypher-highlighting/issues/02-highlighter-cutover-keyword-function.md`
