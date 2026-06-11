# 04 — Transaction episode model + correlated history tags

Status: done

## Parent

`.scratch/workbench-ergonomics-2/PRD.md` — introduces the **Transaction episode**
(`CONTEXT.md`).

## What to build

Make a query run inside an explicit transaction reviewable as *what it was* — the
Nth statement of an episode that was ultimately committed or undone.

- Track a Session-scoped **Transaction episode**: an episode number that
  increments on each `:begin`, and a per-statement **ordinal** that increments on
  each submission made while the Transaction is Open. Because all Buffers share
  the one open Transaction, statements submitted from different Buffers belong to
  the same episode and continue its ordinal sequence.
- **Tag each Result-history entry** submitted while Open with its episode and
  ordinal, and a **disposition**: `open` initially, resolved to `committed` or
  `rolled back` when `:commit` / `:rollback` ends the episode. Resolution is
  **retroactive across every Buffer's history** — all entries of the ending
  episode flip from `[open]`. A poisoned-then-rolled-back transaction resolves to
  `rolled back`.
- **Render** the tag in the results-pane header alongside the query snippet, e.g.
  `tx 2 · stmt 3 [open]` → `[committed]` / `[rolled back]`. A rolled-back entry
  keeps its rows but is clearly marked as undone. Autocommit entries are untagged.

Demo: `:begin`; run two queries; `:rollback` — both history entries now read
`tx N · stmt k [rolled back]` with their rows still visible but flagged undone.

## Acceptance criteria

- [ ] Each submission made while a transaction is Open records its episode number
      and ordinal; ordinals continue across Buffers within one episode.
- [ ] History entries render `tx N · stmt k [disposition]` in the results-pane
      header; autocommit entries carry no transaction tag.
- [ ] `:commit` / `:rollback` flips every matching episode entry's disposition in
      all Buffers' histories, retroactively.
- [ ] A rolled-back entry still shows its rows, marked as undone.
- [ ] Reducer tests cover ordinal assignment across Buffers, retroactive
      disposition resolution, and the poisoned→rolled-back path.

## Blocked by

None - can start immediately.
