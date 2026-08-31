# 12 — `:o` one-shot output redirection + shared format vocabulary

Status: done

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

- [x] `:o <file>` writes the next query's result to the file via the existing
      format writers, then output returns to the screen (one-shot).
- [x] Format is inferred from the file extension, overridable by an explicit
      `:o <format> <file>`; an unknown format/extension reports a clear error.
- [x] A shared `csv | jsonl | cypherl | table` vocabulary (`OutputFormat`, with
      `FromStr`/`Display`/`from_extension`) is used by the batch `--output-format`
      flag, the Workbench export, and `:o` (one definition, three callers).
- [x] Redirection streams the Record stream row-by-row to the file (bounded
      memory) for csv/jsonl/cypherl; `table` buffers as the documented exception.
- [x] An unwritable path / failed query reports a clear error without ending the
      session.

## Blocked by

None - can start immediately.

## Comments

Implemented (AFK).

- **Shared vocabulary** (`cli/src/lib.rs`): the existing `OutputFormat` is now the
  one vocabulary — variant `Tabular` renamed to `Table` (with `tabular` accepted
  as a flag alias and by `FromStr`), plus `FromStr`/`Display`/`from_extension`/
  `is_streaming`. Three callers: the `--output-format` flag, the Workbench export
  gesture (its old `ExportFormat` enum removed, prompt cycles the streaming subset
  via `next_export_format`), and `:o`.
- **REPL** `:o`: `parse_redirect` (explicit `csv|jsonl|cypherl|table` token or
  extension inference), a `run_to_file` `QueryRunner` method, and a one-shot
  `pending_redirect` armed by `:o` and consumed by the next query (also works in
  `:source`). Streams via the core `RowWriter`s; `table` buffers.
- **Workbench** `:o`: `state.redirect` armed by the command; the next submit's
  first statement runs via `Effect::RunQueryToFile` → an async `stream_query_to_file`
  (generic over the writer so the spawned task stays `Send`) → `Event::Redirected`,
  reusing the run-state/advance lifecycle. The rest of the submission runs normally.
- A bare-endpoint `:o` to an unknown extension, and write failures, are reported
  without losing the session.
