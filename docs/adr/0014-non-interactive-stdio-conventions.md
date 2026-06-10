# 14. Non-interactive stdio: TTY-selected defaults, `jsonl` on piped stdout, data-to-stdout / chrome-to-stderr

Date: 2026-06-10

## Status

Accepted

## Context

ADR 0002 gave the Core a streaming-first set of render formats (`tabular`,
`csv`, `jsonl`, `cypherl`) and named non-interactive scripting — run a file or a
single query, machine-readable output, CI exit codes — as a first-class goal.
ADR 0010 then selected the Frontend by `stdin().is_terminal()`: a TTY gets an
interactive Frontend (the TUI workbench by default, the line REPL behind
`--plain`), while a non-TTY stdin routes to the non-interactive piped path.

What those ADRs did *not* settle is the *conventions* of that piped path now that
it is the primary scripting surface for a tool meant to be reached for instead of
the legacy `mgconsole` (ADR 0013). Piping forces three questions the interactive
Frontends never had to answer: what a non-TTY **stdin** means, what **stdout**'s
default format should be, and which stream carries data versus chrome. A graph
console makes the format question pointed: a Memgraph Value is frequently nested
(node, relationship, path, map, list), and ADR 0003's mandate is to render every
Value *faithfully* — which a flat `csv` cannot do for a node.

## Decision

Treat stdin and stdout as **two independent TTY checks**.

- **stdin selects the Frontend (interactive ↔ batch), per ADR 0010.** TTY stdin
  → interactive (TUI default, REPL via `--plain`). Non-TTY stdin → the script
  runner reading stdin. A bare `... | mgconsole` therefore runs stdin as
  `cypherl`; it is exactly equivalent to the explicit `mgconsole run -`, where
  `-` is the conventional stdin sentinel for `run`. The explicit `run -` is the
  form to prefer in committed scripts and CI, where behaviour should not hinge on
  whether something happens to be a tty. `mgconsole -c "QUERY"` runs one query
  and exits.

- **stdout selects the default render format.** This is the result-display-mode
  Setting (resolved by precedence: built-in default < config < CLI flag <
  runtime `:set`; `--format` overrides), but the **built-in default is itself
  TTY-aware**: TTY stdout → `tabular`, non-TTY stdout → **`jsonl`** (one JSON
  object per Record). `jsonl` is chosen over `csv`/`tsv` because Values nest and
  must render faithfully (ADR 0003); it also pipes straight into `jq`. This
  reuses ADR 0002's existing streaming `jsonl` writer — no new format.

- **Stream discipline: stdout carries only Query-result data.** The prompt,
  Notifications, errors, and `:`-command output all go to **stderr**. This is
  what makes every pipe compose regardless of Frontend, and lets `mgconsole >
  out` (interactive in, redirected out) log clean `jsonl` while the human still
  sees prompts and Notifications.

- **Batch exit codes.** `run` / `run -` exit non-zero on query error, satisfying
  ADR 0002's CI-exit-codes goal so the tool composes under `set -e`. Because
  stdin is an ordinary `cypherl` source, the Import modes apply unchanged:
  `--serial` (default, streams), `--parallel` (buffers Batches; needs
  vertices-first ordering), `--parser`.

## Consequences

- `... | mgconsole | jq` works out of the box, and `... | mgconsole` runs a
  piped script with zero ceremony — the ergonomics a "goto" scripting tool needs.
- The result-display Setting now carries a TTY-aware *default* beneath the rest
  of its precedence chain; a future reader must not flatten it to a single static
  default, or piping breaks.
- Nothing in the Core changes: this is conventions over ADR 0002's formats and
  ADR 0010's Frontend selection, not new rendering machinery.
- **Kept deliberately distinct:** stdin-as-*queries* (`run -`, decided here) is
  not stdin-as-*data* (NDJSON rows bound as Query parameters, or a bulk-load
  stream). The latter is a separate later feature and is explicitly *not* implied
  by bare-pipe, so it cannot be conjured by accident. Likewise optional/later:
  `... | mgconsole tui` seeding the first Buffer from stdin and reopening
  `/dev/tty` to interact.
