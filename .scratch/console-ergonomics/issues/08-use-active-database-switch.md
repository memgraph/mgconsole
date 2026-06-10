# 08 — `:use` = active-Database switch (multi-tenancy)

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add `:use <db>` to switch the active Database (`CONTEXT.md`) within the current
Session — same Endpoint, same connection — using Memgraph multi-tenancy
(`USE DATABASE`). Switching Databases refetches the Schema, so completion and the
schema sidebar reflect the newly-active Database. The active Database is shown in
the prompt and status bar alongside the Endpoint/profile.

## Acceptance criteria

- [x] `:use <db>` switches the active Database on the same Session (no
      reconnect) via `USE DATABASE`. The success path needs an Enterprise license
      (Community has no multi-tenancy); the wiring is verified against a live DB
      through the error path (see note).
- [x] The Schema refetches for the newly-active Database after a switch (the
      Workbench emits `FetchSchema` on success; the REPL has no schema source).
- [x] The active Database appears in the prompt and status bar.
- [x] Switching to an unknown/unavailable Database reports a clear error and
      leaves the current Database active.
- [x] `:use` behaves identically in the REPL and Workbench.

## Blocked by

- `.scratch/console-ergonomics/issues/07-connect-session-swap.md`

## Comments

Implemented (AFK).

- **Core** (`core/src/session.rs`): `Session::use_database(db)` runs
  `USE DATABASE <db>` on the same Session and drains the empty result; a failure
  surfaces as `Error::Query` leaving the current Database active.
- **Frontends**: `:use <db>` parses into the shared `MetaCommand`. REPL — the
  runner switches and updates the prompt (`memgraph@<endpoint>/<db> …`). Workbench
  — `Effect::UseDatabase` → the edge switches and, on success, re-fetches the
  Schema (so completion + the sidebar reflect the new Database) and emits
  `Event::DatabaseChanged`; the status bar shows `<endpoint>/<db>`. A `:connect`
  swap resets the active-database display to the new Session's default.
- **Probe finding / note**: against Memgraph 3.10.1 Community, `USE DATABASE`
  (and `SHOW DATABASES`) return "enterprise feature without a valid license", so
  the *success* path can't be exercised here. A live integration test
  (`use_database_surfaces_a_clear_error_and_keeps_the_session_usable`) covers the
  on-the-wire error path and Session survival; the success path is structurally
  identical (same drain-and-return), exercised by the unit-tested Frontend wiring.
