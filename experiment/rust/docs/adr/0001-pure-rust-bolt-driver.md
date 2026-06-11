# 1. Pure-Rust Bolt driver over FFI to mgclient

Date: 2026-06-10

## Status

Accepted — **confirmed by the fidelity spike (2026-06-10)**: against
`memgraph:3.10.1` at Bolt v4.4, all 21 Memgraph Value types decode faithfully
through `bolt-client` 0.11 + `bolt-proto` 0.12 with **no codec extension or
fork required**; the `mgclient` FFI fallback is not triggered. See
`.scratch/rust-console/spike/FINDINGS.md`.

## Context

The Rust console must speak the Bolt protocol to Memgraph. The C++ `mgconsole`
uses Memgraph's own `mgclient` C library, which guarantees exact Memgraph
semantics — every value type (enums, spatial points with SRID, the full
temporal family, paths) round-trips correctly because it is the library the
server team maintains.

Three options were considered:

1. **FFI to `mgclient`** via bindgen — maximal fidelity and protocol parity,
   but keeps a C toolchain and OpenSSL in the build, an `unsafe` boundary, and
   a single-threaded blocking model.
2. **A pure-Rust Bolt stack** — `bolt-client` (connection/handshake) plus
   `bolt-proto` (PackStream values). No C dependencies, async-native (tokio),
   `rustls` TLS, trivial static `musl` builds.
3. **Hand-rolling the Bolt codec** — full control, most effort, we own protocol
   maintenance forever.

A console's job is narrow: handshake, run a query with parameters, stream
Records back as raw typed Values, and render them. It needs no ORM-style
abstraction. Working at the low `bolt-proto` layer means the server's Values
pass through as raw PackStream structures we render ourselves, which mirrors
`mgconsole`'s "print whatever comes back" design.

The known risk: `bolt-proto` and the wider pure-Rust Bolt ecosystem target
Neo4j. Memgraph emits custom PackStream struct signatures (notably enums,
points, and some temporal encodings) that a Neo4j-shaped decoder may not know
and could reject or mis-decode. For a tool whose entire purpose is faithful
rendering, a silent type loss is the worst outcome.

## Decision

Build on the **pure-Rust `bolt-client` + `bolt-proto` stack**, working at the
raw-Value layer rather than a high-level driver such as `neo4rs`.

Before committing the main build, run a verification spike: connect to a real
Memgraph and return every Value type — enum, point, date, localtime,
localdatetime, duration, zoned datetime, node, relationship, path, nested
list/map — and confirm each decodes faithfully. Where `bolt-proto` lacks a
Memgraph struct signature, extend or fork the codec to add it. Teaching the
codec Memgraph's signatures is accepted as in-scope work.

## Consequences

- No C toolchain, no OpenSSL; static single-binary builds are trivial.
- The driver is async (tokio), which aligns with the chosen runtime
  (see ADR 0002).
- We own a thin layer of Memgraph-specific PackStream decoding and must track
  it as Memgraph's Bolt surface evolves — a smaller, well-bounded maintenance
  burden than hand-rolling the whole codec.
- The fidelity risk is front-loaded into a spike, not discovered late. If the
  spike shows the gap is larger than a few struct signatures, this decision is
  the one to revisit (FFI to `mgclient` remains the fallback).
