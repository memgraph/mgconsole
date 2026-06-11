# 18. Yank prefers a local clipboard helper process, falling back to OSC 52

Date: 2026-06-11

## Status

Accepted — refines ADR 0015

## Context

ADR 0015 set the Workbench yank (`y`/`Y`) to copy via OSC 52 and "never a native
clipboard library", reasoning that the tool is "very often driven over SSH to a
bastion" where a native clipboard cannot reach a display, and that OSC 52 is pure
bytes (no FFI, ADR 0001) that rides the SSH/tmux channel. ADR 0015 accepted one
cost explicitly: "on a terminal that does not honour OSC 52, the yank silently
does nothing useful."

That accepted cost turned out to be the *common local case*, not a corner. A
grilling session reproduced it: a **local** Wayland session (`WAYLAND_DISPLAY`
set, not SSH, not tmux) on a VTE-family terminal (`TERM=xterm-256color`) that
does not honour OSC 52. Yank reported "copied" but the system clipboard was never
set, and there was no way to make that terminal work — and the SSH justification
that motivated OSC 52 did not even apply.

The constraint that shaped 0015 still holds: on Wayland there is **no
zero-dependency, pure-Rust, no-FFI way** to set the system clipboard. The only
mechanisms are (a) a terminal that honours OSC 52, (b) an external helper binary
(`wl-copy`, `xclip`, `xsel`, `pbcopy`, `clip.exe`), or (c) a native FFI crate
(`arboard`) — and (c) is forbidden by ADR 0001 (pure Rust, no unsafe). Crucially,
shelling out to a helper binary is **not** a native clipboard *library*: it is a
`std::process::Command` writing to a child's stdin — pure bytes, no FFI, no new
crate — so it sits inside ADR 0001's boundary, and was simply not considered in
0015.

## Decision

**Yank tries a local clipboard helper process first, and falls back to OSC 52.**

- On copy, probe for a helper in order — `wl-copy` → `xclip` → `xsel` → `pbcopy`
  → `clip.exe` — and pipe the rendered text to the first one present. If none is
  found, emit OSC 52 as before.
- The **status line reports the path taken**: `copied (wl-copy)` when a helper
  succeeded; `copied (OSC 52 — paste may need terminal support)` when it fell back,
  so a silent no-op becomes a visible, explained outcome.
- This covers all cases: a local box gains a real clipboard once a helper is
  installed (`apt install wl-clipboard`); SSH-without-helper still works via OSC 52
  (ADR 0015's decisive win is preserved); a helper-less, OSC-52-less terminal at
  least *says* what it did.

## Consequences

- **ADR 0001 stays intact**: a helper subprocess is no FFI and no platform crate.
  ADR 0015's "never a native clipboard *library*" also stands — a process is not a
  library. This ADR refines 0015's *mechanism ordering*, it does not overturn its
  no-FFI principle.
- A fully-local clipboard now requires an external binary to be present. The tool
  does not bundle or install one; the status message points the user at it. We
  accept a one-time install over re-opening the FFI boundary.
- The helper probe is best-effort and cached per session; a helper that exists but
  fails (e.g. no compositor) still falls back to OSC 52.
- The copied text is still the *rendered* Value (ADR 0015, unchanged) — the helper
  change is about *where the bytes go*, not *what bytes*.
