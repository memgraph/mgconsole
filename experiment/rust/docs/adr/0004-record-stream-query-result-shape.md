# 4. Owned RecordStream and QueryResult, summary readable after drain

Date: 2026-06-10

## Status

Accepted

## Decision

The Session returns a `QueryResult { header, records: RecordStream, summary }`,
where `RecordStream` is a Core-owned type (it does **not** expose
`futures::Stream` in Core's public API) and `summary` — timing, notifications,
and stats — is only readable once the records have been drained. This shape is
established in issue 02 (even though the tracer bullet only populates `header`
plus one record), so that issue 13 fills `summary` and issue 21 exploits the
streaming records for bounded memory, both as additive changes rather than
signature migrations.

## Consequences

- Keeping `futures::Stream` out of the public surface insulates Core's callers
  from the `futures` version and leaves us in control of the
  `block_on`-at-the-boundary ergonomics ADR 0002 mandates (tabular `block_on`s a
  collect; streaming writers `block_on` a drive-loop). The cost is that we forgo
  the ready-made `Stream` combinator ecosystem and maintain a small stream type.
- `summary`-after-drain mirrors Bolt itself: the server sends `RECORD` messages
  and then a final `SUCCESS` carrying the metadata. A consumer that wants stats
  must consume or discard the records first; this is a protocol fact, not a
  limitation we chose.
- Issue 02 carries the whole `QueryResult` shape before it is justified, so it
  reads as slightly over-built until 13 and 21 land. Accepted deliberately to
  avoid migrating the Session's core return type twice.
