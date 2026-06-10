# 19 — TTY-aware default output format + stream discipline

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md` — implements **ADR 0014** (non-interactive
stdio conventions).

## What to build

Make the **default** result format depend on whether stdout is a terminal, and
pin the stream discipline that makes pipes compose.

Today the output format is a static default of `tabular` regardless of where
stdout points, so `… | mgconsole | jq` emits a box-drawing table. Per ADR 0014,
when no format is chosen explicitly the default is **`tabular` on a TTY stdout**
and **`jsonl` on a non-TTY stdout** (piped/redirected). `jsonl` is the right
non-interactive default because Memgraph Values nest (nodes/rels/paths/maps) and
must render faithfully (ADR 0003) — a flat format cannot. An explicit
`--output-format` (and any future config/`:set` layer) still wins: this changes
only the built-in default, which now sits *beneath* the existing precedence chain
(default < config < flag) and is itself a function of stdout.

Alongside it, assert the **stream discipline**: stdout carries *only*
Query-result data; everything else — prompt, Notifications, reconnect notices,
errors, and `:`-command/echo output — goes to stderr. Most of this already holds
in the serial path; this slice makes it explicit and covers the `mgconsole > out`
case (interactive stdin, redirected stdout), where result data must land in the
file as the resolved non-TTY format while prompts/Notifications stay on the
terminal via stderr.

## Acceptance criteria

- [ ] With no `--output-format`, a non-TTY stdout defaults to `jsonl` and a TTY
      stdout defaults to `tabular`.
- [ ] An explicit `--output-format` overrides the TTY-derived default in both
      directions (e.g. `--output-format tabular` into a pipe still tabulates).
- [ ] `… | mgconsole | jq` consumes the output without a format flag.
- [ ] Only Query-result data is written to stdout; Notifications, reconnect
      notices, errors, and any echo/chrome go to stderr.
- [ ] `mgconsole > out` writes result data to the file in the resolved non-TTY
      format while prompts/Notifications remain visible on the terminal.
- [ ] The default-resolution is unit-tested as a pure function of (explicit flag,
      stdout-is-tty), with no terminal required — in the `ColorChoice::resolve`
      style used for frontend/colour selection.

## Blocked by

None - can start immediately.
