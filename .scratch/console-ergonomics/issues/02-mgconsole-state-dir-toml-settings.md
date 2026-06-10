# 02 — `~/.mgconsole` state directory + TOML `[settings]` loader

Status: ready-for-agent

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

- [ ] History resolves under `~/.mgconsole` (the bare default), with
      `MGCONSOLE_HISTORY_PATH` still winning; resolver comments no longer claim
      `~/.memgraph` mgconsole compatibility.
- [ ] `~/.mgconsole/config.toml` `[settings]` values load and sit between
      built-in defaults and the CLI flag in the precedence chain (verified by a
      pure resolution test).
- [ ] `MGCONSOLE_CONFIG_PATH` overrides the config-file location; a missing
      config file is not an error (defaults apply).
- [ ] A malformed `config.toml` is reported with a clear message and the console
      still starts on defaults.
- [ ] `display` (and any other issue-01 setting) is honoured when set in
      `config.toml`.

## Blocked by

- `.scratch/console-ergonomics/issues/01-set-mechanism-vertical-display.md`
