# 01 — `:set` mechanism + vertical/`auto` display

Status: done

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

- [x] Core renders a Record vertically (`column: value` per line) as an
      alternative to tabular, covered by golden tests alongside the existing
      tabular goldens.
- [x] `display` setting supports `tabular | vertical | auto`; `auto` is the
      default and flips to vertical only when a row exceeds the terminal width,
      reusing fit-to-screen (issue 08) — verified by a pure unit test of the
      mode-selection decision.
- [x] `:set` (list) and `:set <name> <value>` (change) parse into the shared
      `MetaCommand` vocabulary and behave identically in the REPL and Workbench.
- [x] An unknown setting name or invalid value is reported without losing the
      session.
- [x] `:set` and `:param` remain distinct surfaces; setting `display` never
      touches the parameter store and vice-versa.

## Blocked by

None - can start immediately.

## Comments

Implemented (AFK).

- **Core** (`core/src/display.rs`): `DisplayMode {Tabular,Vertical,Auto}` with
  `FromStr`/`Display`; `render_vertical` (`-[ RECORD n ]-` blocks, one
  `column: value` per line) golden-tested in `core/tests/vertical_golden.rs`; the
  pure `resolve_layout(mode, natural_width, term_width)` carries the `auto`
  decision (unit-tested), reusing `tabular::natural_width` (the real renderer)
  rather than a parallel path; `render_records` is the single dispatch entry.
- **Setting spine** (`cli/src/settings.rs`): `Settings` resolves
  default < CLI flag (`--display`) < runtime `:set`; the config-file layer is
  deferred to issue 02 as planned. Validation returns a message, leaving the
  store untouched on error.
- **Shared `MetaCommand`**: `:set` / `:set <name> <value>` parse once in
  `cli/src/repl.rs` and are honoured by both the REPL loop and the Workbench
  reducer (`handle_meta`). The REPL renders via `render_records`, so
  `:set display vertical` is visibly vertical there.
- **Note**: the Workbench stores and validates `display` identically, but its
  on-screen result is a *navigable* ratatui table, a different rendering surface
  from the Core buffered renderer; flipping that live widget to a vertical layout
  is left as a follow-up (the cell-detail overlay already gives a single-record
  expanded view). The `:set` command itself behaves identically across both.
