# 09 — Export on-screen result

Status: done

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Take the current result out of the workbench. An export action writes the
on-screen result to a file in csv / jsonl / cypherl, reusing the Core's existing
format writers (no new rendering). The user picks the format and destination; the
result reflects what is loaded (and, for a cancelled query, the partial rows,
clearly the case). Success/failure is reported in the status.

## Acceptance criteria

- [x] The current result can be exported to csv, jsonl, or cypherl via the
      existing Core writers. (`write_export` drives the rows through
      `CsvWriter`/`JsonlWriter`/`CypherlWriter` via the `RowWriter` trait.)
- [x] The export reflects the loaded rows (including a partial result after a
      cancel); the outcome is reported in the status. (`confirm_export` clones the
      on-screen `result.rows`; `ExportFinished` sets the status.)
- [x] Format selection and the export effect are covered at the reducer seam.
      (open/format-cycle/confirm, blank-path, Esc-cancel, finished-outcome tests.)

## Blocked by

- `.scratch/tui-workbench/issues/02-nonblocking-execution-spine.md`
