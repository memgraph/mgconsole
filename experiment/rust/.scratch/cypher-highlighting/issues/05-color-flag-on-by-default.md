# 05 — `--color` flag, on by default

Status: done

## Parent

`.scratch/cypher-highlighting/PRD.md`

## What to build

Turn interactive input colouring on by default and put it behind a standard
flag, per ADR 0009. Replace the `--term-colors` boolean with
`--color=auto|always|never`:

- `auto` (the default) — colour on for an interactive terminal.
- `always` — force colour on.
- `never` — force colour off.

Honour the `NO_COLOR` environment convention. Precedence: an explicit `--color`
beats `NO_COLOR`, which beats the `auto` default. Highlighting only ever happens
in the interactive REPL; the piped/non-interactive path never colours regardless,
so no colour can leak into machine-readable output.

The pinned test asserting colour is off-by-default and opt-in
(`term_colors_is_off_by_default_and_opt_in`, `cli/src/lib.rs`) inverts to assert
the new on-by-default behaviour and the `--color` resolution.

## Acceptance criteria

- [ ] `--color` accepts `auto` / `always` / `never`, defaulting to `auto`.
- [ ] With `auto` on an interactive terminal, colour is on without any flag.
- [ ] `NO_COLOR` set disables colour, unless overridden by `--color=always`.
- [ ] `--color=never` disables colour; `--color=always` forces it on.
- [ ] The old `--term-colors` flag is gone; the default-behaviour test is
      inverted and passes, along with tests for the precedence rules.

## Blocked by

- `.scratch/cypher-highlighting/issues/02-highlighter-cutover-keyword-function.md`
