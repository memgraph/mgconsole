# 14 — Theme + keybinding config (Workbench)

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Make the Workbench's colours and key chords configurable from `config.toml`. A
`[theme]` table maps the existing frontend-neutral HighlightCategory values
(slice 04 of tui-workbench) and UI element colours; a `[keys]` table rebinds the
Workbench gestures' chords. `:set theme <name>` switches between built-in themes
at runtime. Defaults reproduce today's appearance and bindings, so an empty
config changes nothing. Keybinding config is the home for the tab chords (issue
18) and other gestures.

## Acceptance criteria

- [ ] `[theme]` in `config.toml` overrides Workbench colours via the
      HighlightCategory seam (no new colour vocabulary); an empty/absent table
      reproduces today's appearance.
- [ ] `[keys]` rebinds Workbench gesture chords; conflicting/unknown bindings are
      reported clearly and fall back to defaults.
- [ ] `:set theme <name>` switches between built-in themes at runtime and is
      listed by `:set`.
- [ ] Theme/keys are resolved through the issue-02 config precedence (defaults <
      config file < runtime `:set` for theme).
- [ ] Covered by tests at the config-resolution seam (no terminal needed).

## Blocked by

- `.scratch/console-ergonomics/issues/02-mgconsole-state-dir-toml-settings.md`
