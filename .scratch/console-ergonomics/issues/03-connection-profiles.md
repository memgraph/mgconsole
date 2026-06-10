# 03 — Connection profiles + `--profile` + active-profile indicator

Status: done

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

- [x] `[profiles.<name>]` in `config.toml` parses into a profile carrying
      endpoint, auth, TLS, `readonly`, and setting overrides.
- [x] `--profile <name>` selects a profile; its endpoint/auth/TLS drive the
      connection and its setting overrides apply with config-file precedence.
- [x] The active profile name appears in the REPL prompt and the Workbench
      status bar; with no profile selected, neither shows a stale name.
- [x] An unknown `--profile` name fails fast with a clear message listing the
      known profiles.
- [x] CLI connection flags still work with no profile, and explicit flags
      override a selected profile's values (flag > config-file precedence).

## Blocked by

- `.scratch/console-ergonomics/issues/02-mgconsole-state-dir-toml-settings.md`

## Comments

Implemented (AFK).

- **Parsing** (`cli/src/config.rs`): `[profiles.<name>]` → `Profile {host, port,
  username, password, use_ssl, readonly, settings}` (nested `[settings]` mirrors
  the top-level table); `Config::select` fails fast listing known names.
- **`--profile`** (`cli/src/lib.rs`): flag added; `resolve_connection` folds
  flag > profile > default. To tell an explicit flag from a clap default, `main`
  parses via `ArgMatches` and reads `value_source` into `ExplicitFlags` — so an
  unset flag takes the profile's value and an explicit flag always wins. The
  profile's `[settings]` overlay (`FileSettings::overlay`) sits at the config-file
  precedence layer.
- **Indicator**: REPL prompt becomes `memgraph (<name)> ` (`repl_prompt`);
  Workbench status bar shows a stable `[<name>]` prefix. Neither shows a stale
  name with no profile.
- **`readonly`** is parsed and carried on `Connection`, but enforcement (Bolt
  READ access mode) is issue 04 — left as a clean seam, not yet wired to the
  Session.
