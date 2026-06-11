# 18 — Notifications / stats / verbose-exec-info surfacing

Status: done

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Surface the trailing summary information the Session already computes. A result's
notifications (hints/warnings), update statistics, and — when requested —
verbose execution info (cost, parse, plan, execute times) are shown in the status
area and/or a toggleable summary drawer, distinct from the result data. Reuses
the Core's existing summary/notification computation; no new Core work.

## Acceptance criteria

- [x] Notifications attached to a result are shown to the user (status and/or a
      summary drawer), distinct from the result rows. (`on_completed` notes the
      count in the status; the Ctrl-Y summary drawer lists them in full,
      separate from the table.)
- [x] Update statistics and, when the verbose flag is set, verbose execution info
      are surfaced. (`draw_summary` lists `summary.stats`, and — when
      `--verbose-execution-info` is set — `summary.execution_info()`.)
- [x] The summary surfacing is covered at the reducer seam, reusing the existing
      summary/notification types. (summary-kept + notification-in-status and
      Ctrl-Y toggle reducer tests; a `draw_summary` golden; Core `Summary`/
      `Notification`/`ExecutionInfo` reused.)

## Blocked by

- `.scratch/tui-workbench/issues/02-nonblocking-execution-spine.md`
