# Up-switch: `:workbench` from the REPL, capability-gated

Status: ready-for-agent

## What to build

The return direction, closing the round-trip (ADR 0019). Typing `:workbench` in
the REPL stands up the full-screen Workbench over the same live Session, with one
fresh Buffer.

- **`:workbench` as a Meta-command in the REPL.** Returns `SwitchTo(Workbench)`
  to the dispatch loop, which calls `TerminalGuard::enter()` and runs the
  Workbench over the bundle from issue 01. The arriving Workbench builds **one
  fresh Buffer** (lossy, per ADR 0019 — there is nothing to restore on this
  slice). Idempotent: a no-op ("already in the Workbench") when typed in the
  Workbench. Appears in the REPL `:help` / command vocabulary.
- **Capability gate, not the `--plain` flag.** `:workbench` is allowed only when
  the terminal can host the TUI (`supports_tui`: a capable tty plus the `tui`
  feature). It is **independent of how the run started** — launching `--plain` on
  a capable terminal and upgrading with `:workbench` must work, because `--plain`
  was a startup preference, not a permanent verdict. The runtime needs the
  `supports_tui` signal available in the REPL to make this check.
- **Graceful refusal.** On an incapable terminal (`TERM=dumb`, non-tty),
  `:workbench` is refused with a clear message and never crashes into a broken
  alternate screen. In a build compiled without the `tui` feature the command
  does not exist at all (it is feature-gated, like the Workbench Frontend
  variant).
- **`:repl` stays the universal floor** — always available, never capability-checked.

## Acceptance criteria

- [ ] `:workbench` typed in the REPL on a capable terminal lands in the Workbench
      with the connection, open Transaction, params, Settings, active Database,
      and Read-only mode all intact, in one fresh Buffer.
- [ ] A full round-trip — Workbench → `:repl` → `:workbench` — returns to a
      working Workbench with the Session preserved (tabs/history lost, as
      designed).
- [ ] Launching with `--plain` on a capable terminal, then `:workbench`, switches
      successfully (the `--plain` startup flag does not block the upgrade).
- [ ] On an incapable terminal (`TERM=dumb` / non-tty), `:workbench` is refused
      with a clear message and the terminal is left usable.
- [ ] In a build without the `tui` feature, `:workbench` is not a recognised
      command.
- [ ] `:workbench` typed in the Workbench is a no-op with an "already in the
      Workbench" message.
- [ ] `:workbench` appears in the REPL `:help` / command vocabulary.

## Blocked by

- Issue 01 (down-switch carries the dispatch loop, the outcome enum, and the
  Session-state bundle this slice re-enters the Workbench over).
