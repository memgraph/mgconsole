# 16 — In-result search/filter (Workbench)

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add a Workbench gesture to search and filter rows within the currently-displayed
result. A search chord opens a small query input; matching cells are highlighted
and the user can jump between matches, with a filter toggle to show only matching
rows. It operates over the rows already materialised in the result view (it does
not re-run the query or pull more from the server). Openly Workbench-only.

## Acceptance criteria

- [ ] A search chord filters/highlights rows in the displayed result by substring
      match across cells; clearing search restores the full view.
- [ ] Next/previous-match navigation moves the selection between matches.
- [ ] A filter toggle shows only matching rows; toggling off restores all loaded
      rows.
- [ ] Search operates only over already-loaded rows (no re-query, no extra pull);
      this is stated in the status when the result is partial.
- [ ] Reducer-level tests cover match, navigate, filter, and clear.

## Blocked by

None - can start immediately.
