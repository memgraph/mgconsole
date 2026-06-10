# 03 — Error wrap constructors

Status: ready-for-agent

## Parent

`.scratch/architecture-deepening/PRD.md`

## Problem

The same error-wrapping closure is repeated across seven sites, with the
`Connection` vs `Protocol` choice made per-site:

- `core/src/session.rs`: `run_once` (RUN reply, and the post-`FAILURE` `RESET`),
  `reset`, `establish` (`Client::new` → `Connection`; HELLO → `Protocol`)
- `core/src/result.rs`: `pull_batch`, `discard`

```rust
.map_err(|e| Error::Connection(e.to_string()))?   // ×6
.map_err(|e| Error::Protocol(e.to_string()))?     // ×1
```

A blanket `From<bolt_client::Error>` can't work — the variant choice is
context-dependent — so the wrap shape is retyped each time.

## What to build

Inherent constructors on `Error` (`core/src/error.rs`) that carry the
stringify-and-wrap, leaving the variants as `(String)`:

```rust
impl Error {
    pub(crate) fn connection(e: impl std::fmt::Display) -> Self {
        Error::Connection(e.to_string())
    }
    pub(crate) fn protocol(e: impl std::fmt::Display) -> Self {
        Error::Protocol(e.to_string())
    }
}
```

### Migration

- The seven `map_err` sites become `.map_err(Error::connection)?` /
  `.map_err(Error::protocol)?`.
- The existing `format!`-built sites (`reconnect`'s terminal error, the
  "unexpected … reply" / "invalid TLS server name" messages) may either call
  `Error::connection(format!(…))` or keep constructing the variant directly —
  whichever reads cleaner. No behaviour change required there.

### Out of scope

Do **not** restructure the variants to retain the source error (`#[source]` /
`Box<dyn Error>`). That ripples into every `match` on `Error::Connection(_)`
(notably the reconnect branch in `run_with_params`) and is a separate decision.
This issue is pure dedup with no behaviour change.

## Acceptance criteria

- [ ] `Error::connection` and `Error::protocol` exist, taking `impl Display`, variants unchanged
- [ ] All seven `map_err(|e| Error::X(e.to_string()))` sites use the constructors
- [ ] `Error` variants still hold `String`; no match on `Error::Connection(_)` needed changing
- [ ] Behaviour unchanged; existing tests pass (`cargo test`)

## Coordination

Touches `core/src/error.rs`, `core/src/session.rs`, `core/src/result.rs`.
Overlaps issues 01 (`result.rs`) and 02 (`session.rs`). Apply sequentially —
landing this **last** is easiest, since the constructors slot into whatever the
other two leave behind.
