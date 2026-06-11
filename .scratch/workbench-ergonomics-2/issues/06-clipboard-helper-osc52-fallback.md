# 06 — Clipboard helper process + OSC 52 fallback

Status: ready-for-agent

## Parent

`.scratch/workbench-ergonomics-2/PRD.md` — implements **ADR 0018** (yank prefers a
local clipboard helper process, OSC 52 fallback).

## What to build

Make yank actually reach the system clipboard on a local box whose terminal does
not honour OSC 52, without re-opening the FFI boundary (ADR 0001).

- On `y` / `Y` (cell / row, unchanged from ADR 0015), probe for a local clipboard
  helper in order — **`wl-copy` → `xclip` → `xsel` → `pbcopy` → `clip.exe`** — and
  pipe the rendered text to the first one present via `std::process::Command`
  (no FFI, no new crate). If none is found, **fall back to emitting OSC 52** as
  today (the SSH / no-helper path; ADR 0015's decisive win is preserved).
- The **status line reports the path taken**: `copied (wl-copy)` on a helper
  success; `copied (OSC 52 — paste may need terminal support)` on fallback — so a
  silent no-op becomes a visible, explained outcome.
- The copied text stays the *rendered* Value (ADR 0015, unchanged); only where the
  bytes go changes. The helper probe is best-effort; a helper that exists but
  fails falls back to OSC 52.

Demo: with `wl-clipboard` installed, `y` copies a cell and pasting in another app
works, status reads `copied (wl-copy)`; uninstall it and yank falls back to OSC 52
with the explanatory status.

## Acceptance criteria

- [ ] Yank pipes the rendered payload (cell vs row) to the first available helper
      among `wl-copy`/`xclip`/`xsel`/`pbcopy`/`clip.exe` at the IO edge.
- [ ] With no helper present, yank falls back to the OSC 52 sequence.
- [ ] The status line reports which path was used.
- [ ] No native clipboard crate / FFI is added (ADR 0001 / ADR 0018 honoured).
- [ ] The reducer emits a copy effect carrying the exact payload; the helper-probe
      and fallback selection are covered by tests at the IO seam.

## Blocked by

None - can start immediately.
