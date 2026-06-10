# 12 — `:o` one-shot output redirection + shared format vocabulary

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add `:o` to redirect a result to a file, reusing the streaming format writers
from slices 22–24 rather than a new output path. `:o <file>` redirects the **next
query's** result then returns output to the screen — a one-shot, not a sticky
toggle (a toggle you can forget is a footgun, inconsistent with the tool's
no-silent-state rulings). The format is inferred from the extension, or stated
explicitly (`:o csv <file>`). Unify the format names into one shared vocabulary —
`csv | jsonl | cypherl | table` — used identically by the batch `--output` flag,
the Workbench export gesture (slice 09 of tui-workbench), and `:o`. Streaming-first
holds: `:o` tees the live Record stream to the file as it renders to screen, so a
huge result is not buffered.

## Acceptance criteria

- [ ] `:o <file>` writes the next query's result to the file via the existing
      format writers, then output returns to the screen (one-shot).
- [ ] Format is inferred from the file extension, overridable by an explicit
      `:o <format> <file>`; an unknown format/extension reports a clear error.
- [ ] A shared `csv | jsonl | cypherl | table` vocabulary is used by the batch
      `--output` flag, the Workbench export, and `:o` (one definition, three
      callers).
- [ ] Redirection tees the streaming Record stream (bounded memory), not a
      buffered copy.
- [ ] An unwritable path reports a clear error and the result still renders to
      screen.

## Blocked by

None - can start immediately.
