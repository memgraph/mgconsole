# 08 — `:use` = active-Database switch (multi-tenancy)

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add `:use <db>` to switch the active Database (`CONTEXT.md`) within the current
Session — same Endpoint, same connection — using Memgraph multi-tenancy
(`USE DATABASE`). Switching Databases refetches the Schema, so completion and the
schema sidebar reflect the newly-active Database. The active Database is shown in
the prompt and status bar alongside the Endpoint/profile.

## Acceptance criteria

- [ ] `:use <db>` switches the active Database on the same Session (no
      reconnect), verified against a live multi-tenant database.
- [ ] The Schema (completion sources and the Workbench schema sidebar) refetches
      for the newly-active Database after a switch.
- [ ] The active Database appears in the prompt and status bar.
- [ ] Switching to an unknown Database reports a clear error and leaves the
      current Database active.
- [ ] `:use` behaves identically in the REPL and Workbench.

## Blocked by

- `.scratch/console-ergonomics/issues/07-connect-session-swap.md`
