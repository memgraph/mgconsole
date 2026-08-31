# 22 — Scaffold a commented example `config.toml` on interactive first run

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

On the first interactive run, when no `config.toml` exists yet, write a
fully-commented example file to the resolved config path so users discover the
format (profiles, settings, `[theme]`, `[keys]`) without reading source or docs.
This is purely a discoverability scaffold: the template is **entirely commented
out**, so it has zero effect on behaviour until the user edits it — a freshly
scaffolded file parses to `Config::default()`, exactly as a missing file does
today.

Mirror the existing state-dir pattern: `history.rs::prepare_history_dir`
(`create_dir_all` for `~/.mgconsole`, called from `open_history`) is the model.
Add a sibling `config::prepare_config_dir` (or reuse the same ensure-dir helper)
and a write-if-missing step.

### Trigger conditions (all must hold)

- The frontend is an interactive TTY (not a piped / `run -` / NDJSON / `-c`
  one-shot invocation) — a headless or scripted run must **never** create a file
  as a side effect.
- `MGCONSOLE_CONFIG_PATH` is unset, i.e. we resolved the default
  `~/.mgconsole/config.toml`. An explicit override path is the user's to manage;
  don't scaffold into it.
- The resolved config file does not already exist.

### Behaviour

- Write the embedded template (a `const &str` in `config.rs`) to the path,
  creating `~/.mgconsole` if needed.
- Announce it once on stderr (chrome, not data): e.g.
  `wrote example config to ~/.mgconsole/config.toml`.
- A write/permission failure is a warning, not fatal — the console still starts
  on defaults, matching how `prepare_history_dir` and a malformed `config.toml`
  are handled today.
- The template content should track the hint-commented example we settled on:
  a commented `[profiles.local]` (localhost / 7687 / `use_ssl = false`) plus
  commented `username`/`password`/`readonly`, per-profile `[settings]`, global
  `[settings]`, `[theme]`, and `[keys]`. Keep it the single source of truth for
  "what a config looks like".

## Acceptance criteria

- [ ] First interactive run with no config writes a commented
      `~/.mgconsole/config.toml`; the message is printed to stderr.
- [ ] The scaffolded file parses to `Config::default()` (template is inert until
      edited) — covered by a test that parses the embedded `const`.
- [ ] A second run does not overwrite an existing file (no clobber).
- [ ] Non-interactive invocations (pipe, `run -`, `-c`, NDJSON) never write the
      file, even when it is missing.
- [ ] `MGCONSOLE_CONFIG_PATH` set → no scaffold, regardless of TTY.
- [ ] A write failure (e.g. unwritable dir) warns and the console still starts.

## Design notes

- **ADR 0012** (`~/.mgconsole` state directory) is the relevant decision: this
  adds first-run *population* of that directory but does not change its location
  or the env-override escape hatch. No new ADR needed; if we want the scaffold
  behaviour recorded, amend 0012's Consequences rather than add a new record.
- Embedding the template as a `const` (not reading a packaged file) keeps the
  binary self-contained and lets a unit test parse it directly.
- Decisions captured from review: trigger = interactive first-run only; content
  = fully-commented template; record as an issue before coding.

## Blocked by

- `.scratch/console-ergonomics/issues/02-mgconsole-state-dir-toml-settings.md` (done)
- `.scratch/console-ergonomics/issues/03-connection-profiles.md` (done)
- `.scratch/console-ergonomics/issues/14-theme-keybinding-config.md` (done)
