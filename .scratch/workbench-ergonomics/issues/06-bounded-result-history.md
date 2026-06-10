# 06 — Bounded Result history

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics/PRD.md`

## What to build

Cap the per-Buffer Result history so a long session does not grow unbounded in
memory. Keep the most recent *N* entries in full; for entries older than that,
drop their Records (the memory cost) but keep the lightweight correlation — the
originating query, the summary/Notifications, and any error — so older entries
remain navigable as a record of *what ran*, even though their rows are no longer
held. The cap is a generous default (the row-cap already bounds a single result;
this bounds the *number* of results).

Builds on the entry shape from the correlated Result history (issue 05).

## Acceptance criteria

- [ ] A Buffer holds at most *N* full entries (with Records); older entries keep
      their query / summary / error but drop their Records.
- [ ] Navigating to a trimmed entry shows its query and summary/error with a clear
      "rows no longer held" note instead of a table.
- [ ] The newest *N* entries always keep their Records.
- [ ] Reducer tests cover trimming at the cap and that trimmed entries keep their
      correlation (query + summary/error).

## Blocked by

- `.scratch/workbench-ergonomics/issues/05-correlated-result-history.md`
