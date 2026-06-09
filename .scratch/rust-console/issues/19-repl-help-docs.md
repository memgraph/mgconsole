# 19 — `:help` and `:docs` commands

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The informational REPL commands: `:help` prints interactive-mode usage (the
supported commands and how queries work), and `:docs` points the user to
Memgraph documentation. Carry over the content from `mgconsole`'s usage text.

## Acceptance criteria

- [ ] `:help` prints usage covering query entry and the supported `:` commands
- [ ] `:docs` prints the documentation pointers
- [ ] Both return to the prompt without running a query
- [ ] Help text lists the commands actually implemented (help, quit, docs, param, params)

## Blocked by

- `.scratch/rust-console/issues/16-repl-execute-loop.md`
