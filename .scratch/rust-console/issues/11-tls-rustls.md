# 11 — TLS via rustls

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Encrypted connections over Bolt using `rustls` (no OpenSSL), driven by the
use-ssl flag. Integration tested against a `memgraph/memgraph` container
configured for SSL.

## Acceptance criteria

- [ ] use-ssl establishes an encrypted Bolt connection via rustls
- [ ] A plaintext connection still works when use-ssl is off
- [ ] TLS handshake failures produce a clear error, not a panic
- [ ] No OpenSSL/C dependency is introduced
- [ ] Integration test connects with SSL to an SSL-configured container

## Blocked by

- `.scratch/rust-console/issues/09-cli-flag-surface.md`
