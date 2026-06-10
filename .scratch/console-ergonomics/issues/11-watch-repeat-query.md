# 11 — `:watch [interval] [query]` re-run on a timer

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add `:watch [interval] [query]` to re-run a query on a timer, reusing existing
seams rather than building new ones. With the query omitted it re-runs the last
query; the interval is optional with a 2s default (matching `watch(1)`). It stops
on Ctrl-C / any key, reusing the cancellation seam (ADR 0002 / slice 07) — no new
interrupt mechanism. Each tick redraws in place (clear-and-redraw in the REPL,
repaint the result pane in the Workbench) through the existing fit-to-screen
logic (slice 08), so it is a snapshot view, not an accumulating stream. It is
**refused inside an open transaction** (ADR 0011 — a repeating timer holding a
transaction open is a footgun).

## Acceptance criteria

- [x] `:watch` re-runs the last query every 2s by default; `:watch <interval>`
      and `:watch <interval> <query>` override the interval and/or query (pure
      `parse_watch`, unit-tested).
- [x] Each tick redraws in place as a snapshot, not appended (REPL clears the
      screen and re-renders; the Workbench replaces the previous result rather
      than accumulating).
- [x] A key stops watching and returns to the prompt (REPL: press Enter;
      Workbench: any key, reusing the cancellation seam to free the Session).
- [x] `:watch` is refused with a clear message while an explicit transaction is
      open.
- [x] Behaves in both Frontends.

## Blocked by

- `.scratch/console-ergonomics/issues/05-explicit-transactions-state.md`

## Comments

Implemented (AFK).

- **Parser** (`cli/src/repl.rs`): `parse_watch(args, last_query)` →
  `WatchSpec {interval, query}`. A leading interval token (`2`, `2s`, `500ms`,
  `1.5s`) sets the interval, the rest is the query; with no query the last one is
  reused. Unit-tested across all combinations.
- **REPL**: a `watch` `QueryRunner` method; the production `SessionRunner::watch`
  loops — clear screen (ANSI), run+render the snapshot, then `recv_timeout` on a
  one-shot stdin-reader thread so **pressing Enter stops** it. No crossterm/raw
  mode (works in the lean REPL-only build) and no `unsafe`. Refused in a tx.
- **Workbench**: a `WatchState {query, period_ticks, remaining}`; the existing
  120ms render tick (`TICK_MS`) drives the countdown, re-running on elapse and
  **replacing** the previous snapshot (history doesn't grow). Any key stops it
  (cancelling an in-flight re-run); a `[watch]` marker shows in the status bar.
  Refused in a tx / while a query runs.
- **Note**: "stops on Ctrl-C" is realised as Enter in the plain REPL — true
  any-key/Ctrl-C interception needs raw mode (crossterm), which is gated behind
  the `tui` feature; the Workbench (which has crossterm) stops on any key.
