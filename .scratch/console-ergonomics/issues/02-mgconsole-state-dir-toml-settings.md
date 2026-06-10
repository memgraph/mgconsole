# 02 — `~/.mgconsole` state directory + TOML `[settings]` loader

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Move the console's state to a single directory, `~/.mgconsole`, and add the
config-file layer of the Setting precedence chain (ADR 0012). History moves from
`~/.memgraph/client_history` to `~/.mgconsole/client_history` as a clean switch
(no migration shim) — update the slice-17 resolver and its compatibility
comments. Add a `config.toml` reader that loads a `[settings]` table and feeds it
into the precedence chain between built-in defaults and the CLI flag, so
default < config file < CLI flag < runtime `:set` holds. Add a
`MGCONSOLE_CONFIG_PATH` env override alongside the existing
`MGCONSOLE_HISTORY_PATH`. Path/precedence resolution stays a pure, filesystem-free
function as in slice 17.

## Acceptance criteria

- [x] History resolves under `~/.mgconsole` (the bare default), with
      `MGCONSOLE_HISTORY_PATH` still winning; resolver comments no longer claim
      `~/.memgraph` mgconsole compatibility.
- [x] `~/.mgconsole/config.toml` `[settings]` values load and sit between
      built-in defaults and the CLI flag in the precedence chain (verified by a
      pure resolution test).
- [x] `MGCONSOLE_CONFIG_PATH` overrides the config-file location; a missing
      config file is not an error (defaults apply).
- [x] A malformed `config.toml` is reported with a clear message and the console
      still starts on defaults.
- [x] `display` (and any other issue-01 setting) is honoured when set in
      `config.toml`.

## Blocked by

- `.scratch/console-ergonomics/issues/01-set-mechanism-vertical-display.md`

## Comments

Implemented (AFK).

- **State dir** (`cli/src/history.rs`): default moved `~/.memgraph` →
  `~/.mgconsole` (constants, the `--history` flag default, comments, tests); the
  `MGCONSOLE_HISTORY_PATH` override is unchanged. The one remaining `.memgraph`
  mention is the deliberate ADR-0012 "clean break from" note.
- **Config loader** (`cli/src/config.rs`): pure `resolve_config_path`
  (`MGCONSOLE_CONFIG_PATH` override < `~/.mgconsole/config.toml`) and a `[settings]`
  TOML reader via serde-derived raw types with `deny_unknown_fields`. Each value
  is a string validated through the setting's own `FromStr`, so the file and
  runtime `:set` share one vocabulary. Missing file → empty overlay; malformed
  file / unknown key / invalid value → clear path-naming error.
- **Precedence** (`cli/src/settings.rs`): `FileSettings` overlay +
  `Settings::resolve(file, cli_flag)` give built-in default < config file < CLI
  flag < runtime `:set`, covered by a pure resolution test.
- **Wiring** (`cli/src/main.rs`): `load_config()` resolves + reads the file,
  warning and falling back to defaults on error so the console always starts.
- Added `serde` + `toml` deps to `cli/Cargo.toml`.
