# 04 — Header type at the format seam

Status: done

## Parent

`.scratch/architecture-deepening/PRD.md`

## Problem

Column names travel as a bare `&[String]` that the format writers re-pair with
row values by index. The `JsonlWriter` is the only writer that needs the pairing,
and it re-zips defensively:

```rust
// core/src/format/jsonl.rs
let key = self.header.get(i).cloned().unwrap_or_else(|| i.to_string());
```

The `i.to_string()` fallback is **dead defensive code**: the header and the row
are co-derived from one query, so they cannot realistically differ in length.
The real friction is primitive obsession — the same `&[String]` header threads
through `write_stream`, every `RowWriter::write_header`, and `render_table` (the
buffered tabular path) with no cohesive type.

`Header` is not a new term — `CONTEXT.md` already defines a Query result as
having "its header (column names)". This issue gives that concept a type. No
`CONTEXT.md` change.

## What to build

A `Header` type for column names, scoped to the **format seam only**.

- A newtype over the column names (e.g. `Header(Vec<String>)` or borrowing —
  implementer's choice) with:
  - a constructor from `&[String]` / `Vec<String>`,
  - access to the names (for CSV's header row),
  - a safe pairing helper: `header.zip(row: &[Value]) -> impl Iterator<Item = (&str, &Value)>`
    that yields only as many pairs as there are names (no positional fallback).
- Change the seam to speak `Header`:
  - `RowWriter::write_header(&mut self, header: &Header)`
  - `write_stream(writer, header: &Header, stream)`
  - `render_table(header: &Header, rows, opts)` (`core/src/tabular.rs`)
- `JsonlWriter::write_row` builds its object via `header.zip(row)`; the
  `unwrap_or_else(|| i.to_string())` fallback is deleted.
- `CsvWriter` writes its header row from the `Header`'s names; `CypherlWriter`
  continues to ignore it.
- Callers (`core/src/import.rs` and/or `cli/src/main.rs` wherever `write_stream`
  / `render_table` are invoked) wrap `QueryResult.header()` into a `Header` at the
  call site.

### Out of scope — do not touch

- `QueryResult.header()` stays `-> &[String]`; do **not** thread `Header` through
  `QueryResult` or `Session`. The reach is the format + tabular seam only.

## Acceptance criteria

- [ ] A `Header` type exists in `core/src/format` with name access and a length-safe `zip(&[Value])`
- [ ] `RowWriter::write_header`, `write_stream`, and `render_table` take `&Header`
- [ ] `JsonlWriter` pairs via `Header::zip`; the `i.to_string()` fallback is gone
- [ ] `CsvWriter` header row comes from the `Header`; `CypherlWriter` unchanged in behaviour
- [ ] `QueryResult` / `Session` signatures are untouched; callers wrap the header at the seam
- [ ] Output for all four formats is byte-identical to before (golden tests still pass)
- [ ] Unit test for `Header::zip` (pairs names with values in order)

## Coordination

Isolated to `core/src/format/` + `core/src/tabular.rs` and the `write_stream` /
`render_table` call sites. Independent of issues 01–03; can run in its own
worktree.

## Comments

- Done in commit `91947a3` on `plan/rust-console`. All acceptance criteria met; `cargo test`, `cargo clippy --workspace --tests`, and `cargo fmt --check` clean.
