# 01 — Frontend selection + TUI shell, lifecycle & editor

Status: ready-for-human

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

The walking skeleton of the workbench Frontend and the seam everything else
builds on (ADR 0010). A pure resolver maps `(--plain flag, stdin is a terminal,
terminal supports the full-screen UI)` to the chosen Frontend: the workbench for
a capable interactive terminal, the line REPL when `--plain` is set or the
terminal cannot support the UI; the piped/non-interactive path is selected as
today and bypasses both.

The workbench enters the alternate screen in raw mode, draws a stacked layout —
a multiline editor pane (a text-input widget with free cursor movement,
selection, undo), a results area, and a one-line status bar — and restores the
terminal cleanly on quit **and on panic**. Enter is wired as the submit gesture
and a universal newline key (shown in the status hint) inserts a newline, so
multiline editing works from the first slice; actual query execution arrives in
slice 02. Esc / Ctrl-D / `:quit` leaves the workbench.

This slice establishes the **pure workbench state + `update(state, event) ->
(state, effects)` reducer** and the thin ratatui-draw / terminal-lifecycle edge,
mirroring how the REPL's `run_loop` is split from its IO. It is **HITL**: the
reducer/effects shape and the panic-safe lifecycle are the foundation the other
17 slices depend on and warrant a design review before fan-out.

## Acceptance criteria

- [ ] A pure Frontend-selection resolver chooses workbench / REPL from
      `(--plain, is_tty, supports_tui)`; `--plain` and an incapable terminal both
      fall back to the REPL; the piped path is unaffected. Covered by unit tests.
- [ ] Launching on a capable terminal enters the alternate screen and draws the
      editor / results / status layout; the editor accepts multiline input with
      free cursor editing and a universal newline key surfaced in the status hint.
- [ ] Esc / Ctrl-D / `:quit` exits and the terminal is restored; a panic also
      restores the terminal (no corrupted shell).
- [ ] `--color`/`NO_COLOR` resolves colour within the workbench independently of
      Frontend selection (a `--color=never` workbench is monochrome, not
      disabled).
- [ ] The workbench state and reducer are exercised by tests with no terminal and
      no database (event-in, state-out), the way `run_loop` is.
- [ ] ratatui/crossterm are behind a Cargo feature; the default binary builds
      without them.

## Blocked by

- None - can start immediately
