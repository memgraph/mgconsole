# 03 — Connection profiles + `--profile` + active-profile indicator

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add Connection profiles (`CONTEXT.md`): named bundles of the where and how of a
connection, stored as `[profiles.<name>]` in the hand-edited `config.toml`. A
profile carries the Endpoint, authentication, transport security, `readonly`,
and per-profile Setting overrides. A `--profile <name>` CLI flag selects one at
launch; its values feed the connection and the Setting precedence chain (a
profile's settings sit at the config-file layer). The active profile name is
shown in the REPL prompt and the Workbench status bar so the user always knows
which connection they are on.

## Acceptance criteria

- [ ] `[profiles.<name>]` in `config.toml` parses into a profile carrying
      endpoint, auth, TLS, `readonly`, and setting overrides.
- [ ] `--profile <name>` selects a profile; its endpoint/auth/TLS drive the
      connection and its setting overrides apply with config-file precedence.
- [ ] The active profile name appears in the REPL prompt and the Workbench
      status bar; with no profile selected, neither shows a stale name.
- [ ] An unknown `--profile` name fails fast with a clear message listing the
      known profiles.
- [ ] CLI connection flags still work with no profile, and explicit flags
      override a selected profile's values (flag > config-file precedence).

## Blocked by

- `.scratch/console-ergonomics/issues/02-mgconsole-state-dir-toml-settings.md`
