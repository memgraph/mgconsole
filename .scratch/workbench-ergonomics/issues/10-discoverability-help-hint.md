# 10 — Discoverability: `?` opens help; hint + help reflect the live keys

Status: done

## Parent

`.scratch/workbench-ergonomics/PRD.md`

## What to build

Make the Workbench's gestures discoverable. Bind bare **`?`** (when the editor is
empty, or the results pane is focused) to open the `:help` overlay, and add a
`· ? help` pointer to the status-bar hint. Regenerate the hint and the help
overlay from the **live `[keys]`** so they always show the user's actual chords,
including the remapped Buffer navigation (`Ctrl+PageUp/PageDown`), `Ctrl+L`
format, and the `:close` command. Typing `?` mid-query still inserts the character.

Depends on the chord remap (issue 02) so the displayed chords and the `:close`
command are correct.

## Acceptance criteria

- [ ] `?` with an empty editor, or while the results pane is focused, opens the
      help overlay; typing `?` into a non-empty query still inserts it.
- [ ] The status hint includes a pointer to help and reflects the live chords (not
      a stale fixed string).
- [ ] The help overlay lists the remapped nav/format chords and the `:close`
      command, read from the live `[keys]`.
- [ ] Tests cover the `?` binding (open vs insert) and that the help/hint reflect a
      `[keys]` rebinding.

## Blocked by

- `.scratch/workbench-ergonomics/issues/02-chord-command-remap.md`
