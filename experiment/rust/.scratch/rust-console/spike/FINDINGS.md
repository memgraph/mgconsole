# 01 — Bolt fidelity spike: findings & recommendation

Date: 2026-06-10
Spike code: `.scratch/rust-console/spike/` (throwaway; `cargo run` is self-contained)

## Setup

- **Stack:** `bolt-client` 0.11.0 + `bolt-proto` 0.12.0 (pure Rust, no C, no OpenSSL).
- **Server:** `memgraph/memgraph:3.10.1`, started on demand via `testcontainers`
  0.27 (Docker daemon only; mapped to a random host port).
- **Bolt version:** offered `[V4_4, V4_3, V4_2, V4_1]`; Memgraph 3.10.1 negotiated
  **v4.4** (the highest `bolt-proto` 0.12 speaks). HELLO with `scheme: "none"`
  is accepted on a default (auth-disabled) Memgraph.
- **Method:** one query per Value type, fresh connection each, `RUN` split from
  `PULL` so a server-side Failure (bad Cypher) is distinguishable from a
  client-side decode gap (unknown PackStream struct signature, which surfaces as
  `bolt_proto ... invalid signature byte: XX`).

## Result: every Value type decodes faithfully

**21 / 21 decode · 0 need codec extension · 0 errors.**

| Value type | Outcome | Decoded as |
|---|---|---|
| null | DECODES | `Null` |
| boolean | DECODES | `Boolean(true)` |
| integer | DECODES | `Integer(42)` |
| float | DECODES | `Float(3.14159)` |
| string (unicode) | DECODES | `String("héllo ☃")` |
| list (mixed) | DECODES | `List([Integer, String, Float, Null])` |
| map (nested) | DECODES | `Map({...})` |
| node | DECODES | `Node { node_identity, labels, properties }` |
| relationship | DECODES | `Relationship { rel_identity, start/end, rel_type, properties }` |
| path | DECODES | `Path { nodes, relationships: [UnboundRelationship], sequence }` |
| unbound relationship | DECODES | (carried inside `Path`, decoded cleanly) |
| date | DECODES | `Date(2021-06-15)` |
| localtime | DECODES | `LocalTime(12:34:56.789)` |
| localdatetime | DECODES | `LocalDateTime(2021-06-15T12:34:56.789)` |
| duration | DECODES | `Duration { months: 0, days: 1, seconds: 7384, nanos: 0 }` |
| zoned datetime (offset) | DECODES | `DateTimeOffset(2021-06-15T14:34:56+02:00)` |
| zoned datetime (named tz) | DECODES | `DateTimeZoned(...CEST)` (IANA zone preserved) |
| point 2D cartesian | DECODES | `Point2D { srid: 7203, x, y }` |
| point 2D wgs84 | DECODES | `Point2D { srid: 4326, x, y }` |
| point 3D cartesian | DECODES | `Point3D { srid: 9157, x, y, z }` |
| point 3D wgs84 | DECODES | `Point3D { srid: 4979, x, y, z }` |
| enum | DECODES | `Map({"__type": "mg_enum", "__value": "Status::Active"})` |

The key ADR-0001 risk — that Memgraph emits custom PackStream struct signatures a
Neo4j-shaped decoder rejects — **did not materialise**. Memgraph 3.10.1 at Bolt
v4.4 uses the standard signatures `bolt-proto` already implements for the whole
temporal family and for spatial points (all four SRIDs).

## Recommendation: PROCEED with pure-Rust Bolt (ADR 0001 confirmed)

No codec extension or fork is required. The named `mgclient` FFI fallback is
**not** triggered. The fidelity risk that justified gating the whole build is
retired.

## Findings that feed downstream issues

1. **Enum is a tagged map, not a struct, not a codec gap (→ issue 07).**
   Memgraph transmits an enum value as a plain PackStream map with sentinel keys
   `{"__type": "mg_enum", "__value": "<Enum>::<Member>"}`. `bolt-proto` decodes
   it losslessly as `Value::Map`. Faithful *rendering* is therefore a
   render-layer concern: detect the `__type == "mg_enum"` sentinel and render the
   `__value` (e.g. `Status::Active`) rather than a raw map. This belongs in the
   point/enum rendering slice, not in the transport.

2. **Temporal crate decided: `chrono` (→ issue 06).** `bolt-proto` 0.12 exposes
   temporals as `chrono` types (`NaiveDate`, `NaiveTime`, `NaiveDateTime`,
   `DateTime<FixedOffset>`, `DateTime<Tz>`, plus a `Duration` struct). This
   resolves the PRD's open "`time` vs `chrono`" question in favour of `chrono`,
   because that is what the transport hands us. Note `chrono-tz` for named zones.

3. **Two distinct zoned-datetime Value variants (→ issues 06, and rendering).**
   An *offset* datetime decodes to `Value::DateTimeOffset(DateTime<FixedOffset>)`;
   a *named-zone* datetime decodes to `Value::DateTimeZoned(DateTime<Tz>)`. The
   renderer must handle both arms. (Memgraph uses the legacy pre-Bolt-5
   signatures `0x46`/`0x66` at v4.4, which is exactly what `bolt-proto` expects.)

4. **Points carry their SRID (→ issue 07).** Observed: 2D cartesian `7203`,
   2D wgs84 `4326`, 3D cartesian `9157`, 3D wgs84 `4979`. The renderer should
   surface the SRID / coordinate system rather than just x/y/z.

5. **Node/relationship identities.** `node_identity` / `rel_identity` are present
   as integers; relationships also carry `start_node_identity` /
   `end_node_identity`. `UnboundRelationship` (no endpoints) only appears inside
   `Path`, as expected.

6. **`localDateTime` fractional-seconds gotcha (test-fixture note, not a bug).**
   Memgraph rejects `.5`; it requires exactly 3 or 6 fractional digits
   (`.789` / `.789000`). The validation error is misleadingly wrapped in a
   `Memgraph.TransientError.*` code. Worth remembering when writing temporal
   golden fixtures (issue 06) and the error taxonomy (issue 14): a
   `TransientError` code does **not** reliably mean "retryable".

## Caveats / not covered

- Tested at Bolt **v4.4** only (the `bolt-proto` 0.12 ceiling). Memgraph also
  speaks v5; v5 changed the zoned-datetime signatures. We do not need v5, but if
  a future `bolt-proto` negotiates it, re-verify temporals.
- No TLS in this spike (ADR 0001 names `rustls`; that is issue 11).
- `bytes` (PackStream byte string) not exercised — Cypher has no literal and it
  is not in the acceptance list; `Value::Bytes` exists in `bolt-proto` if needed.
