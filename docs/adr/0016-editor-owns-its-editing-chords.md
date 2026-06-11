# 16. The editor owns the Ctrl/Alt+letter chords it uses for editing

Date: 2026-06-11

## Status

Accepted

## Context

The Workbench shares one keyspace between two consumers: the multiline Cypher
editor (a `tui-textarea`) and the rebindable Workbench gestures (drawer toggles,
search, buffer navigation). Several editor operations are themselves
`Ctrl`/`Alt`+letter chords — delete-word (`Ctrl+W`), undo (`Ctrl+Z`), redo
(`Ctrl+Y`), and the structural `Ctrl+C` (cancel) / `Ctrl+D` (quit) / `Ctrl+J`
(newline). When a Workbench gesture is bound to one of those, it shadows the
editor operation: `Ctrl+W` once closed a Buffer instead of deleting a word, and
`Ctrl+Y` toggled the summary drawer instead of redoing.

A grilling session weighed three readings of "the editor owns single-`Ctrl`/`Alt`
+letter chords": absolute (every letter chord belongs to the editor, so *all*
gestures move to non-letter chords), narrow (the editor owns only the chords it
actually *uses* for editing), and ad-hoc. The absolute reading would force a large
migration of the drawer toggles (`Ctrl+B`/`Ctrl+P`) and search (`Ctrl+F`) that
coexist with editing perfectly well; the narrow reading matches what shipped.

## Decision

**The editor owns the `Ctrl`/`Alt`+letter chords it uses for editing; Workbench
gestures must never bind to them.** The owned set is delete-word (`Ctrl+W`), undo
(`Ctrl+Z`), redo (`Ctrl+Y`), and the structural `Ctrl+C`/`Ctrl+D`/`Ctrl+J`. Other
letter chords (`Ctrl+B`/`Ctrl+P`/`Ctrl+R`/`Ctrl+F`/`Ctrl+T`) remain fair game for
gestures because the editor does not use them.

Redo is `Ctrl+Y` rather than `Ctrl+Shift+Z`: `Ctrl+Y` arrives on any terminal,
whereas `Ctrl+Shift+Z` needs the keyboard-enhancement protocol the tool only
negotiates opportunistically. Because `Ctrl+Y` is now editor-owned, the summary
drawer toggle moved from `Ctrl+Y` to **`Ctrl+N`** (notifications — the drawer's
leading section).

## Consequences

- A future contributor cannot rebind a gesture onto `Ctrl+W`/`Ctrl+Z`/`Ctrl+Y`
  (etc.) without silently breaking editor input — this ADR records why those are
  off-limits. (`[keys]` rebinding is unconstrained, so a user *may* still create
  such a collision deliberately; this governs the built-in defaults.)
- Undo/redo bind globally (no focus-scoping) because the collision is gone — the
  chord means one thing everywhere, which is the predictability the console aims
  for, rather than meaning different things per focused pane.
- The defaults are config-reversible, but the *ownership boundary* is a standing
  rule that guides every future default-chord choice.
