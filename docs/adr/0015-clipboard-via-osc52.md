# 15. Workbench clipboard copy via OSC 52, not a native clipboard library

Date: 2026-06-11

## Status

Accepted

## Context

The Workbench captures mouse events (click to focus/select a cell, scroll, click a
tab), which turns off the terminal's own click-drag text selection — only
`Shift`-drag still selects natively. That removes the one way a user could copy a
Value out of the results, and the Workbench otherwise has no "copy this" action:
cell-expand lets you *see* a wide or nested Value but not extract it. For a graph
console, "copy this id / this JSON" is a daily need.

Two mechanisms could put a Value on the system clipboard:

- A **native clipboard crate** (`arboard`, `copypasta`). Broad desktop coverage,
  but it links platform clipboard APIs (X11/Wayland/AppKit/Win32) — a re-entry of
  the FFI boundary **ADR 0001** spent real effort to forbid (pure-Rust, no unsafe)
  — and it cannot reach a clipboard at all over SSH, where there is no local
  display. A DB console is very often driven over SSH to a bastion, so this is the
  common case, not a corner.
- **OSC 52.** The application writes a terminal escape sequence carrying the
  base64 payload; the *terminal* sets the system clipboard. It is pure bytes — no
  FFI, no platform crate — and because it rides the same channel as the rest of
  the TUI, it works through SSH and `tmux`. The cost is that the terminal must
  support OSC 52: kitty, WezTerm, iTerm2, foot, and `tmux` (with `set-clipboard
  on`) do; a few terminals need it enabled, and some not at all.

## Decision

Copy to the clipboard with **OSC 52**, never a native clipboard library.

- A **yank** gesture in the results pane: `y` copies the selected cell's rendered
  Value, `Y` copies the whole selected row; the cell-detail overlay yanks the same
  way. (`Ctrl+C` is cancel, so the vim-style `y`/`Y` are used.) The copied text is
  the Value rendered exactly as shown.
- `Shift`-drag stays the documented native-selection fallback, and `:set mouse
  off` is the escape hatch for users who would rather have full native selection
  than mouse gestures — it releases mouse capture for the session.

## Consequences

- Copy works over SSH and `tmux`, which a native clipboard crate cannot do — the
  decisive reason given how the tool is reached.
- **ADR 0001 stays intact:** no clipboard FFI, no new platform-specific
  dependency.
- On a terminal that does not honour OSC 52, the yank silently does nothing useful
  — so the status line confirms the yank and `Shift`-drag remains as the manual
  path. We accept this over broad desktop coverage rather than re-open the FFI
  boundary.
- The yank copies *rendered* text (what the user sees), not a re-serialization;
  copying a node as `jsonl`/`cypherl` would be a separate, later choice and is not
  implied here.
