# 11 — `:watch [interval] [query]` re-run on a timer

Status: ready-for-agent

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

- [ ] `:watch` re-runs the last query every 2s by default; `:watch <interval>`
      and `:watch <interval> <query>` override the interval and/or query.
- [ ] Each tick redraws in place via fit-to-screen (a snapshot, not appended);
      verified against a live database.
- [ ] Ctrl-C (or any key) stops watching and returns to the prompt, reusing the
      cancellation seam.
- [ ] `:watch` is refused with a clear message while an explicit transaction is
      open.
- [ ] Behaves in both Frontends (REPL clear-and-redraw; Workbench pane repaint).

## Blocked by

- `.scratch/console-ergonomics/issues/05-explicit-transactions-state.md`
