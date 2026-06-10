# 01 — Typed-key Bolt metadata accessor

Status: ready-for-agent

## Parent

`.scratch/architecture-deepening/PRD.md`

## Problem

The Bolt message metadata map (`HashMap<String, bolt_proto::Value>`) is read by
six places that each re-implement the same `match get(key) { Some(String) => …,
_ => default }` shape, keyed by bare string literals:

- `core/src/proto.rs`: `fields`, `failure_message`, `failure_code`, `has_more`
  (and `query_error`, which composes two of them)
- `core/src/result.rs`: `Notification::from_value` — its own `s = |k| …` closure
  reading `code`/`title`/`description`/`severity` from an inner `Value::Map`
  (which is the **same** `HashMap<String, bolt_proto::Value>` type — confirmed)

A typo in a key (`"cdoe"`) compiles and silently returns the default. The
match-and-default is duplicated; the keys carry no type.

## What to build

A typed-key accessor in `core/src/proto.rs` so the known metadata keys become a
closed, typed schema and the match-and-default lives in exactly one place.

- A borrowing newtype `Meta<'a>` wrapping `&'a HashMap<String, bolt_proto::Value>`.
- A typed key: a `Key<T>` carrying the key string and its value type, plus a
  small extraction trait so `meta.get(KEY)` returns `Option<T>` projecting the
  right `bolt_proto::Value` variant. Implement extraction for the value types
  actually needed: `String`, `bool`, and `Vec<String>` (for `fields`, a list of
  strings).
- A closed set of `const` key definitions (e.g. `FIELDS: Key<Vec<String>>`,
  `CODE: Key<String>`, `MESSAGE: Key<String>`, `HAS_MORE: Key<bool>`, and the
  notification keys `TITLE`/`DESCRIPTION`/`SEVERITY` — `CODE` can be shared).
- A way to wrap an inner `Value::Map` as a `Meta` (e.g. `Meta::child(key)` or a
  `from_map`/`TryFrom` helper) so `Notification` reads its own map through the
  same accessor.

### Migration

- Rewrite `proto::{fields, failure_message, failure_code, has_more}` to read
  through `Meta` + typed keys. **Per-key defaults stay in these domain readers**
  (e.g. `failure_message` keeps `"unknown query error"`, `failure_code` keeps
  `""`, `fields` keeps empty `Vec`). The accessor returns `Option<T>`; the
  reader applies the domain default. Keep their existing `pub(crate)` signatures
  and behaviour.
- Rewrite `Notification::from_value` (`core/src/result.rs`) to wrap its inner map
  in `Meta` and read the four fields via `Key<String>`, dropping the bespoke
  `s = |k| …` closure. Empty-string-on-absence behaviour is preserved.

### Out of scope — do not touch

- `Summary::absorb_terminal`'s `stats` read and `lift_metadata` — those are a
  whole-map `Value::from` translation, not keyed extraction.
- `Summary::execution_info` / `as_f64` — they read an already-translated
  `BTreeMap<String, Value>` (a different layer).
- The `notifications` **list navigation** in `absorb_terminal` stays as-is; only
  each item's map now reads through `Meta` (that falls out of the `Notification`
  migration).

## Acceptance criteria

- [ ] `Meta<'a>` + `Key<T>` + extraction trait exist in `proto.rs` with impls for `String`, `bool`, `Vec<String>`
- [ ] The known metadata keys are defined once as `const` keys; no bare string literal keys remain in the migrated readers
- [ ] `proto::{fields, failure_message, failure_code, has_more, query_error}` read through `Meta`; their `pub(crate)` signatures and default behaviour are unchanged
- [ ] `Notification::from_value` reads through `Meta`; the standalone closure is gone; behaviour unchanged
- [ ] `stats`, `lift_metadata`, and `execution_info` are untouched
- [ ] Unit tests cover the accessor: present key → value, missing key → `None`, wrong-variant → `None`, and the inner-map (`Notification`) path
- [ ] Existing tests pass (`cargo test`)

## Coordination

Touches `core/src/proto.rs` and `core/src/result.rs`. Overlaps issue 03 in
`result.rs` (`pull_batch`/`discard` `map_err`). Apply sequentially.
