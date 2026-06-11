# 18 — Multiple buffers/tabs (Workbench)

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add Buffers (`CONTEXT.md`): tabs in the Workbench, each an independent line of
inquiry owning **its own editor text and its own result + result-history stack**
(slice 10 of tui-workbench). All Buffers share the **single Session**, so the
connection, active profile, Read-only mode, open Transaction, `:param` store,
Schema, and Settings are session-global — and **only one query is ever live
across all Buffers** (a second submit while one is running is refused with a clear
status, never queued or silently cancelled; cross-tab parallelism is a non-goal,
ADR 0006 owns concurrency). The open-transaction and profile/read-only state show
in the status bar regardless of which Buffer is active, keeping the shared state
honest. Buffers are ephemeral (not persisted across runs). There is always at
least one Buffer; closing the last is a no-op. A tab bar shows the Buffers; new /
close / next / prev are Workbench gestures whose chords live in the keybinding
config (issue 14).

## Acceptance criteria

- [ ] New / close / next / prev gestures manage Buffers; a tab bar shows them and
      the active one; always ≥1 Buffer (closing the last is a no-op).
- [ ] Each Buffer keeps its own editor text and its own result + result-history;
      switching Buffers preserves both.
- [ ] Session-global state (connection, profile, read-only, open transaction,
      `:param`, Schema, settings) is shared across all Buffers and reflected in
      the status bar regardless of the active Buffer.
- [ ] Submitting in a Buffer while any Buffer has a live query is refused with a
      clear status (one-live-result guard), not queued or cancelled.
- [ ] Buffer management is covered at the reducer seam; Buffers are not persisted
      across runs.

## Blocked by

- `.scratch/console-ergonomics/issues/14-theme-keybinding-config.md`
