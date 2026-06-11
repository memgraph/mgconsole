# 17 — Persisted command-history recall

Status: done

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Recall earlier queries across sessions in the workbench, reusing the REPL's
persisted history (the existing history-file resolution, `--history` /
`--no-history` / env override, and the file-history wrapper). Submitted queries
are appended to history; the user can recall prior entries into the editor (a
history-recall gesture and/or a searchable history view).

## Acceptance criteria

- [x] Prior entries load on start and submitted queries are appended, honouring
      `--history` / `--no-history` / the env override — the same resolution the
      REPL uses. (`main` opens the history file via `open_history` for both
      Frontends; the edge loads it into a `FileHistory`, sends `HistoryLoaded`,
      and `record`s each `AppendHistory`.)
- [x] The user can recall a previous query into the editor. (Ctrl+Up / Ctrl+Down
      step older/newer through `history_entries`, replacing the editor buffer and
      restoring the saved live buffer past the newest.)
- [x] History recall state is covered at the reducer seam; the existing
      history-resolution helpers are reused unchanged. (`HistoryLoaded`, recall
      walk, append-on-submit, and live-buffer-preserved tests; `history.rs` reused
      verbatim.)

## Blocked by

- `.scratch/tui-workbench/issues/01-frontend-selection-shell-lifecycle.md`
