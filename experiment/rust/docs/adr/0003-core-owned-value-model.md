# 3. Core-owned Value model, translated at the Bolt boundary

Date: 2026-06-10

## Status

Accepted

## Decision

The Core defines its own `Value` type, and the Session translates each
`bolt_proto::Value` into it at the Bolt boundary, rather than letting renderers
consume `bolt_proto::Value` directly. Memgraph-specific semantics are normalised
once, here — most importantly the enum, which Memgraph transmits as a tagged map
(`{"__type": "mg_enum", "__value": "Status::Active"}`; see the ADR-0001 spike)
and which becomes a first-class `Value::Enum` in the Core model.

## Consequences

- Enum detection, and any other transport quirk, lives in exactly one place; the
  four renderers (tabular, csv, jsonl, cypherl) all match on the Core `Value` and
  stay in sync by construction.
- The renderers are insulated from `bolt-proto`'s representation and from Bolt
  version churn — notably the Bolt v5 change to the zoned-datetime signatures the
  spike flagged — at the cost of one translation pass per Record.
- "Core owns the Value model" (ADR 0002) is now literally true: the Core Value
  type, not `bolt_proto::Value`, is the currency every Frontend and renderer
  speaks.

To minimise later refactors, the model is **total from the outset and never
carries a raw form**:

- The Core `Value` enumerates every Memgraph type the ADR-0001 spike found, as
  variants, from the first cut — even those not yet rendered. The spike removed
  all discovery risk, so the variant list is fixed now; only rendering grows
  slice by slice.
- The boundary always normalises: a value is in its semantic Core form the first
  time it is translated (the enum is `Value::Enum`, never a passed-through map
  promoted later). There is no raw→semantic migration because no raw form ever
  ships.
- Renderers match the Core `Value` **exhaustively, with no wildcard arm**, so a
  new variant is a compile error at each render site — guided, mechanical change
  rather than a silent gap.
