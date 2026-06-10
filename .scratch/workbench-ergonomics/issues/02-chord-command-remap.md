# 02 — Chord/command remap: free `Ctrl+W`, `:close`, `Ctrl+PageUp/Down` nav

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics/PRD.md`

## What to build

Move Workbench gestures off the chords the editor owns, and make closing a tab a
deliberate command. The editor keeps the single-`Ctrl`+letter and `Alt`+letter
keyspace; the colliding gesture defaults change (the mechanism — rebindable
`[keys]` — does not):

- **`Ctrl+W` is freed** (no longer close-buffer): the sharpest footgun, since it
  is *delete-word* everywhere else and currently destroys a Buffer mid-edit.
- **Buffer navigation → `Ctrl+PageDown` (next) / `Ctrl+PageUp` (prev)**; **new
  buffer stays `Ctrl+T`** (low collision, non-destructive).
- **Close buffer becomes a Workbench command `:close`** (`CONTEXT.md` "Workbench
  command") — typed and deliberate because it is destructive; the line REPL, which
  has no Buffers, reports `:close` as unavailable.
- **Auto-format moves to `Ctrl+L`** (free in a TUI — no scrollback to clear).

The defaults shift; all gesture chords stay rebindable through `[keys]`.

## Acceptance criteria

- [ ] `Ctrl+W` no longer closes a Buffer (it reaches the editor like any other
      key).
- [ ] `Ctrl+PageDown`/`Ctrl+PageUp` switch to the next/previous Buffer;
      `Ctrl+T` opens a new one.
- [ ] `:close` closes the active Buffer (the last-Buffer-stays rule holds); the
      REPL reports `:close` as unavailable/unknown.
- [ ] `Ctrl+L` auto-formats the editor buffer.
- [ ] The default `[keys]` reflect the new chords and rebinding still works;
      keybinding-resolution tests are updated.

## Blocked by

None - can start immediately.
