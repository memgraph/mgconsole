# 01 — Modal command line; editor submit is Cypher-only

Status: done

## Parent

`.scratch/workbench-ergonomics-2/PRD.md` — implements **ADR 0017** (modal command
line; the Buffer editor holds only Cypher).

## What to build

Give the Workbench a modal **Command line** (`CONTEXT.md`) for the typed
`:`-vocabulary, and stop the Buffer editor from accepting `:`-commands.

- A one-line modal prompt opens on **`:` while the Buffer editor is empty or
  whitespace-only** (mirroring the REPL's "line starts with `:`" rule) and on
  **`Ctrl+G`** anywhere, so a command can be issued mid-composition without
  disturbing the editor. `Ctrl+;` is *not* used (unreliable delivery — same reason
  ADR 0016 rejected `Ctrl+Shift+Z`).
- The command line runs its content on **Enter** through the existing
  `MetaCommand` / Workbench-command path, dismisses on **Esc**, and returns focus
  to where it was. It is not part of the focus cycle.
- **Editor submit no longer routes `:`-lines to the meta parser.** A `:`-line
  typed or pasted into the editor is Cypher (and will error). Once any Cypher is
  present, `:` typed in the editor is literal (labels, rel-types, maps, enums).

Demo: with an empty editor, `:` opens the prompt and `:begin` / `:help` run from
it; `MATCH (n:Person)` types `:` literally; `Ctrl+G` opens the prompt with a
half-written query preserved.

## Acceptance criteria

- [ ] `:` on an empty/whitespace editor opens the command line with `:` present;
      `:` after any Cypher is inserted literally.
- [ ] `Ctrl+G` opens the command line from any focus without altering the editor
      buffer; Esc closes it and restores prior focus.
- [ ] Enter in the command line runs the `:`-command via the existing
      MetaCommand / Workbench-command handling (e.g. `:begin`, `:help`, `:close`).
- [ ] Submitting a `:`-line in the editor is treated as Cypher, not a
      meta-command (the old inline routing is gone).
- [ ] Reducer tests cover open-on-empty vs literal-`:`, the `Ctrl+G` opener, run,
      and cancel; the editor-submit-no-longer-meta change is covered.

## Blocked by

None - can start immediately.
