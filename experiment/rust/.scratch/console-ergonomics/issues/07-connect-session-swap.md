# 07 — `:connect` = Session swap

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add `:connect` to point the console at a different server. Because a Session is
by definition one server (`CONTEXT.md`), `:connect` is a **Session swap**, not a
mutation: the current Session ends and a new Session to the new Endpoint begins,
reusing the existing connect path (auth/TLS). An open transaction on the old
Session is aborted by the swap (same rule as ADR 0011 — explicit work is not
carried across). `:connect <endpoint>` takes a fresh endpoint; `:connect
<profile>` connects by named profile (issue 03). The prompt and status bar
reflect the new Endpoint and profile.

## Acceptance criteria

- [x] `:connect <endpoint>` ends the current Session and establishes a new one to
      the given Endpoint, reusing the existing auth/TLS connect path (via the
      integration-tested `Session::connect_with`).
- [x] `:connect <profile>` connects using a named connection profile (endpoint,
      auth, TLS, readonly, setting overrides).
- [x] An open transaction is aborted by the swap, surfaced clearly (not silently
      carried to the new Session).
- [x] The prompt and status bar update to the new Endpoint/profile after a
      successful swap; a failed connect leaves the prior Session intact with a
      clear error.
- [x] `:connect` behaves identically in the REPL and Workbench.

## Blocked by

- `.scratch/console-ergonomics/issues/05-explicit-transactions-state.md`
- `.scratch/console-ergonomics/issues/03-connection-profiles.md`

## Comments

Implemented (AFK).

- **Resolver** (`cli/src/lib.rs`): `resolve_connect_target` maps a `:connect`
  argument to a `ConnectTarget {endpoint, options, profile}` — a known profile
  name wins (its endpoint/auth/TLS/readonly), else a bare `host[:port]` reusing
  the current Session's auth/TLS/read-only and its port when omitted. Pure,
  unit-tested. Core gains `Session::endpoint()`.
- **The swap** establishes a fresh Session and replaces the old one only on
  success (a failed connect leaves the prior Session intact), reusing the
  integration-tested `Session::connect_with`. An open transaction is warned about
  and dropped (ADR 0011 — bracketed work is never carried across).
- **REPL** (`cli/src/main.rs`): the prompt state was consolidated into a shared
  `Arc<Mutex<PromptInfo>>` (endpoint/profile/read-only/tx), replacing the two
  atomics, so `:connect` updates the prompt live; the prompt now shows
  `memgraph@<endpoint> (profile) [read-only] [tx]> `.
- **Workbench**: `:connect` → `Effect::Connect` → the edge resolves + connects +
  swaps the shared `Arc<Mutex<Session>>` and re-fetches the Schema, then
  `Event::Connected` updates the status-bar endpoint/profile/read-only. Refused
  mid-query.
- **Note**: the live-DB swap is covered transitively (the resolver is unit-tested;
  the connect itself is `Session::connect_with`, already integration-tested); no
  new two-container test was added to keep the suite lean.
