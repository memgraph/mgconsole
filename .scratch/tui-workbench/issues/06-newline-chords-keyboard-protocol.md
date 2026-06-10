# 06 — Shift/Ctrl+Enter newline chords

Status: ready-for-agent

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

The ergonomic newline gesture on top of slice 01's universal newline key. At
startup, negotiate the terminal keyboard-enhancement protocol; on terminals that
support it, **Shift+Enter / Ctrl+Enter insert a newline** while plain Enter
submits. On terminals that do not, behaviour is unchanged — Enter submits and the
universal newline key (slice 01) remains, shown in the status hint. Enter's
meaning (submit) is invariant across all terminals.

## Acceptance criteria

- [ ] On a keyboard-protocol-capable terminal, Shift+Enter and Ctrl+Enter insert
      a newline; plain Enter submits.
- [ ] On a terminal without the protocol, Enter still submits and the universal
      newline key still works; nothing regresses.
- [ ] Capability is detected at startup (negotiated, not assumed).
- [ ] The key-event → editor-action mapping is covered at the reducer seam for
      both the capable and incapable cases.

## Blocked by

- `.scratch/tui-workbench/issues/01-frontend-selection-shell-lifecycle.md`
