# 02 — Endpoint cohesion type for host + port

Status: ready-for-agent

## Parent

`.scratch/architecture-deepening/PRD.md`

## Problem

Host and port travel as two loose primitives (`host: &str, port: u16`) threaded
through five signatures and stored as two separate `Session` fields, with the
`host:port` rendering re-`format!`'d in the reconnect path. There is no single
place the pair is cohesive.

Sites (`core/src/`):
- `session.rs`: `Session::connect`, `Session::connect_with`, the `host`/`port`
  fields, `establish`, and `reconnect` (re-passes them and `format!`s
  `{host}:{port}`)
- `workers.rs`: `Workers::connect(host, port, …)` forwards to
  `Session::connect_with`
- `transport.rs`: `connect_stream(host, port, use_tls)` — uses `host` for both
  the TCP connect and the TLS `ServerName`
- `cli/src/main.rs`: constructs from `cli.host` / `cli.port` (clap flags)

## What to build

A thin cohesion type `Endpoint` in `core/src/session.rs`, beside `ConnectOptions`
(the *where* next to the *how* — keep them as distinct types; do **not** fold
host/port into `ConnectOptions`).

```rust
pub struct Endpoint { host: String, port: u16 }
impl Endpoint {
    pub fn new(host: impl Into<String>, port: u16) -> Self { ... }
    pub fn host(&self) -> &str { &self.host }
    pub fn port(&self) -> u16 { self.port }
}
impl std::fmt::Display for Endpoint { /* "host:port" */ }
```

- Infallible constructor (port range is already guaranteed by `u16`; no host
  validation — there is no invalid-host case clap hands us that warrants a
  `Result` at every connect site).
- `Display` is the single home for the `host:port` rendering; `reconnect`'s
  error message uses it instead of `format!("{}:{}", host, port)`.

### Migration

- `Session` stores one `endpoint: Endpoint` field (replacing `host`/`port`).
- Signatures become `Session::connect(&Endpoint)`,
  `Session::connect_with(&Endpoint, &ConnectOptions)`,
  `establish(&Endpoint, &ConnectOptions)`,
  `Workers::connect(&Endpoint, &ConnectOptions, count)`.
- `transport::connect_stream` takes the `Endpoint` (or `endpoint.host()` +
  `endpoint.port()`); the TLS `ServerName` derives from `endpoint.host()`.
- `cli/src/main.rs` builds the `Endpoint` once after clap parsing and passes it
  to `Session::connect_with` and `Workers::connect`. Clap keeps its separate
  `--host` / `--port` flags.
- Export `Endpoint` from `core/src/lib.rs` alongside `ConnectOptions`.

This is a **public-API change** (`Session::connect*`, `Workers::connect`
signatures). That is accepted — single workspace, no external consumers.

A glossary entry for **Endpoint** has already been added to `CONTEXT.md`.

## Acceptance criteria

- [ ] `Endpoint` exists in `session.rs` with `new`/`host`/`port`/`Display`, exported from `lib.rs`
- [ ] `Session` holds a single `Endpoint` field; no separate `host`/`port` fields remain
- [ ] `Session::connect*`, `establish`, `Workers::connect`, and `transport::connect_stream` take the `Endpoint` (no loose host/port pairs threaded)
- [ ] The reconnect error message renders the address via `Endpoint`'s `Display`, not an inline `format!`
- [ ] `cli/src/main.rs` constructs the `Endpoint` once and both call sites use it
- [ ] TLS still derives its `ServerName` from `endpoint.host()`
- [ ] Unit test for `Endpoint::Display` → `"host:port"`
- [ ] Existing tests pass (`cargo test`)

## Coordination

Touches `core/src/session.rs`. Overlaps issue 03 (`session.rs` `map_err` sites
and the `reconnect`/`establish` bodies). Apply sequentially.
