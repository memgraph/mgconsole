# 17. A modal command line; the Buffer editor holds only Cypher

Date: 2026-06-11

## Status

Accepted

## Context

The Workbench's Buffer editor was a single dual-purpose surface: it accepted both
Cypher and typed `:`-commands, routing a `:`-only buffer to the Meta-command
parser on submit and everything else to Cypher (`submit()`). Sharing one surface
forced `Tab` to mean two things — trigger completion for the word under the
cursor, or, with nothing to complete, cycle focus to the results pane. A grilling
session named this overload as the concrete confusion ("tab to move panel, and tab
for auto complete is confusing").

The `:` prefix is *not* free in the editor: Cypher uses `:` constantly — node
labels `(n:Person)`, relationship types `[r:KNOWS]`, map keys `{name: 'x'}`, and
enums `Status::Active`. So a command surface cannot simply claim `:` everywhere.

Three input models were weighed: a modal `:` command line (editor becomes
Cypher-only), an always-on command pane (a third box, permanent vertical cost),
and keeping the unified surface and only re-keying focus-switch.

## Decision

**The Workbench gains a modal Command line; the Buffer editor holds only Cypher.**

- The **Command line** (`CONTEXT.md`) is a one-line modal prompt for the typed
  `:`-vocabulary (Meta-commands and Workbench commands). It opens on `:` while the
  editor is empty/whitespace-only — mirroring the REPL's "line starts with `:`"
  rule — and on `Ctrl+G` ("go to command") anywhere, so a command can be issued
  mid-composition without disturbing the editor. Enter runs it; Esc cancels; focus
  returns to where it was. The command line is *not* part of the focus cycle.
- Because the editor is Cypher-only, **`:` typed after any Cypher is literal**
  (you reach `(n:` long after the buffer is non-empty), and **editor submit no
  longer routes to the Meta-command parser** — a `:`-line typed or pasted into the
  editor is Cypher and will error. Meta-commands come from the Command line, or
  `:source` for scripts.
- With the overload gone, **`Tab` means completion alone** in the editor
  (open/cycle candidates). **`Shift+Tab`** cycles to the previous candidate while
  the completion popup is open, and switches focus between editor and results when
  it is closed. `Shift+Tab` (BackTab / `CSI Z`) is chosen because it is delivered
  reliably on every terminal, unlike `Ctrl+Tab`.
- `Ctrl+G` and `Shift+Tab` are reliably-delivered keys; `Ctrl+;` was rejected for
  the mid-composition opener for the same reason `Ctrl+Shift+Z` was rejected in
  ADR 0016 — it needs the keyboard-enhancement protocol the tool only negotiates
  opportunistically.

## Consequences

- The editor's keyspace is simpler and matches ADR 0016's boundary: `Tab` is the
  editor's, `Ctrl+G` is a new non-letter opener that does not intrude on the
  editor-owned letter chords.
- A pasted transaction script (`:begin` … `:commit` interleaved with Cypher) is no
  longer run by submitting it in the editor — that is `:source`'s job. The editor
  is a composition surface, not a script runner. This is a deliberate behavioural
  break from the REPL, which interleaves the two at one prompt.
- The Command line is the Workbench's *presentation* of the cross-frontend
  Meta-command vocabulary (`CONTEXT.md` "Meta-command": each Frontend may present
  its effect in its own way), so no new command vocabulary is introduced — only a
  new surface for the existing one.
- `:`-on-empty is config-fixed behaviour, not a rebindable gesture; `Ctrl+G` and
  the focus-switch follow the `[keys]` rebinding rules like other gestures.
