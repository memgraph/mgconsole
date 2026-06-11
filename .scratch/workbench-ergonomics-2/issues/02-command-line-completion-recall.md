# 02 — Command-line completion + recall

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics-2/PRD.md` — refines the Command line (ADR 0017).

## What to build

Make the modal Command line ergonomic to use:

- **Completion**: `Tab` in the command line completes `:command` names (and
  argument hints where the completion engine already knows them), reusing the
  existing completion engine rather than a second mechanism.
- **Recall**: `Up`/`Down` in the command line walk a **command-history** stack
  (past `:`-commands), kept separate from the Cypher query history that
  `Ctrl+Up/Down` recalls into the editor. Stepping past the newest restores the
  in-progress command line.

Demo: open the command line, type `:beg`, `Tab` → `:begin`; press `Up` to recall
the previously-run command.

## Acceptance criteria

- [ ] `Tab` in the command line completes `:command` names from the existing
      completion engine; with no candidates it is a no-op (does not switch focus).
- [ ] `Up`/`Down` recall older/newer past commands; past the newest restores the
      edited line.
- [ ] The command-history stack is distinct from the query history used by
      `Ctrl+Up/Down` in the editor.
- [ ] Reducer tests cover command-name completion and command recall.

## Blocked by

- `.scratch/workbench-ergonomics-2/issues/01-modal-command-line.md`
