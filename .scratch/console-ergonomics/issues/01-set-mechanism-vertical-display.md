# 01 — `:set` mechanism + vertical/`auto` display

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Establish the Setting spine and deliver the first setting end-to-end. Core gains
a **vertical** render mode beside the existing tabular layout — one Record shown
as `column: value` per line — and a `display` Setting selecting `tabular`,
`vertical`, or `auto`. `auto` (the default) renders tabular until a row will not
fit the terminal width, then falls back to vertical, reusing the existing
fit-to-screen logic rather than a parallel path.

Introduce a single `:set` Meta-command: `:set` with no argument lists every
setting and its current value; `:set <name> <value>` changes one. It is parsed
into the shared `MetaCommand` vocabulary and honoured by both the REPL and the
Workbench. Settings resolve by precedence built-in default < CLI flag < runtime
`:set` (the config-file layer arrives in issue 02). Keep `:set` rigorously
separate from `:param`/`:params` (query data), per `CONTEXT.md`.

## Acceptance criteria

- [ ] Core renders a Record vertically (`column: value` per line) as an
      alternative to tabular, covered by golden tests alongside the existing
      tabular goldens.
- [ ] `display` setting supports `tabular | vertical | auto`; `auto` is the
      default and flips to vertical only when a row exceeds the terminal width,
      reusing fit-to-screen (issue 08) — verified by a pure unit test of the
      mode-selection decision.
- [ ] `:set` (list) and `:set <name> <value>` (change) parse into the shared
      `MetaCommand` vocabulary and behave identically in the REPL and Workbench.
- [ ] An unknown setting name or invalid value is reported without losing the
      session.
- [ ] `:set` and `:param` remain distinct surfaces; setting `display` never
      touches the parameter store and vice-versa.

## Blocked by

None - can start immediately.
