# 7. rustls with the ring crypto backend for Bolt TLS

Date: 2026-06-10

## Status

Accepted.

## Context

ADR 0001 chose a pure-Rust Bolt stack and listed "no C toolchain, no OpenSSL"
among its consequences, naming `rustls` as the TLS library. Slice 11 wires
`--use-ssl` to a real encrypted Bolt connection, which forces a concrete choice
of rustls crypto provider:

- **ring** — the long-standing default. No OpenSSL, but it compiles bundled C
  and assembly, so a C compiler is needed at build time. Battle-tested, widely
  deployed, supports static `musl` builds.
- **aws-lc-rs** — rustls' newer default. Also C/assembly.
- **rustls-rustcrypto** — a pure-Rust RustCrypto-backed provider, the only option
  that literally satisfies "no C toolchain". Currently `0.0.2-alpha`: immature,
  unaudited, and slower.

"No OpenSSL" is satisfied by all three. "No C toolchain" is satisfied only by the
alpha provider, which is too immature to stake a tool's ability to connect on.

## Decision

Use **rustls with the ring crypto provider** (`tokio-rustls`, default features
off, `ring` + `tls12` enabled). This keeps OpenSSL out and preserves static
`musl` builds, at the cost of a C compiler in the build chain.

The server certificate is **not verified**. This mirrors mgclient's `REQUIRE`
sslmode that today's `mgconsole` uses with `--use-ssl`: encrypt the channel
without authenticating the peer, so self-signed Memgraph certificates connect
without configuration. Certificate verification can be added later as an opt-in
without changing this decision.

## Consequences

- The "no C toolchain" line in ADR 0001's consequences is relaxed to "no
  OpenSSL"; a C compiler is required to build ring. Revisit if a pure-Rust
  provider matures (`rustls-rustcrypto` reaching a stable release would let us
  drop the C build dependency with no API change).
- TLS authenticates encryption only, not identity — acceptable parity with
  today's tool, but not suitable as-is for untrusted networks where MITM is a
  concern. An opt-in verifying mode is future work.
- `unsafe` stays forbidden: the no-verify verifier uses rustls' safe (if
  `dangerous()`-named) builder API, no `unsafe` block.
