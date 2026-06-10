# 07 — `:connect` = Session swap

Status: ready-for-agent

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

- [ ] `:connect <endpoint>` ends the current Session and establishes a new one to
      the given Endpoint, reusing the existing auth/TLS connect path, verified
      against a live database.
- [ ] `:connect <profile>` connects using a named connection profile (endpoint,
      auth, TLS, readonly, setting overrides).
- [ ] An open transaction is aborted by the swap, surfaced clearly (not silently
      carried to the new Session).
- [ ] The prompt and status bar update to the new Endpoint/profile after a
      successful swap; a failed connect leaves the prior Session intact with a
      clear error.
- [ ] `:connect` behaves identically in the REPL and Workbench.

## Blocked by

- `.scratch/console-ergonomics/issues/05-explicit-transactions-state.md`
- `.scratch/console-ergonomics/issues/03-connection-profiles.md`
