# 12. Console state lives in `~/.mgconsole`, a clean break from mgconsole's `~/.memgraph`

Date: 2026-06-10

## Status

Accepted

## Context

Slice 17 deliberately placed persisted history at `~/.memgraph/client_history`
to match the C++ `mgconsole` (its resolver comments call this compatibility out
explicitly). Adding a config file, connection profiles, and named queries raised
the question of where the console's *own* state should live. Sharing
`~/.memgraph` with the legacy tool risks the two clobbering each other and ties
us to its layout.

## Decision

Keep all of this console's state in a single directory, `~/.mgconsole` —
history, `config.toml` (hand-edited settings and profiles), and `queries.toml`
(tool-managed named queries) together. This is a deliberate break from
mgconsole's `~/.memgraph` compatibility: we would rather coexist cleanly beside
the legacy tool than share its directory.

The switch is clean — no migration shim that reads the old `~/.memgraph` history
— because the tool is pre-release with no real users yet. The `MGCONSOLE_*`
environment overrides remain the escape hatch (e.g. `MGCONSOLE_HISTORY_PATH`,
plus a sibling `MGCONSOLE_CONFIG_PATH`).

## Consequences

- Slice 17's resolver and its compatibility comments must be updated to point at
  `~/.mgconsole`; the abandoned `~/.memgraph` compat is recorded here so a future
  reader doesn't "restore" it.
- One state directory keeps history, config, and named queries coherent rather
  than scattered across two roots.
- If real users ever predate a later move, this clean-switch precedent will not
  apply and a migration path would be needed.
