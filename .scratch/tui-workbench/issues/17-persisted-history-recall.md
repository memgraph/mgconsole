# 17 — Persisted command-history recall

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Recall earlier queries across sessions in the workbench, reusing the REPL's
persisted history (the existing history-file resolution, `--history` /
`--no-history` / env override, and the file-history wrapper). Submitted queries
are appended to history; the user can recall prior entries into the editor (a
history-recall gesture and/or a searchable history view).

## Acceptance criteria

- [ ] Prior entries load on start and submitted queries are appended, honouring
      `--history` / `--no-history` / the env override — the same resolution the
      REPL uses.
- [ ] The user can recall a previous query into the editor.
- [ ] History recall state is covered at the reducer seam; the existing
      history-resolution helpers are reused unchanged.

## Blocked by

- `.scratch/tui-workbench/issues/01-frontend-selection-shell-lifecycle.md`
