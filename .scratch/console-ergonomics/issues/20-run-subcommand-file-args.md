# 20 — `run [FILES…]` / `run -` explicit batch subcommand

Status: ready-for-agent

## Parent

`.scratch/console-ergonomics/PRD.md` — implements **ADR 0014** (non-interactive
stdio conventions).

## What to build

Add an explicit, additive `run` subcommand for the non-interactive path, so a
batch run does not have to rely on shell redirection or on stdin happening to be
a pipe. `mgconsole run <file>…` runs one or more `cypherl` sources in order
through the existing serial path; the conventional `-` means stdin, so
`mgconsole run -` is the explicit spelling of `… | mgconsole`. Bare-pipe
(`… | mgconsole`) stays as sugar for `run -`, unchanged — `run -` is just the
form to prefer in committed scripts and CI, where behaviour should not hinge on
whether something is a tty.

The subcommand is **additive**: the existing top-level `--import-mode` and
`--output-format` flags keep working as today (back-compat, pre-release). `run`
accepts the same batch options (import mode, output format, csv options) and
inherits the TTY-aware default from issue 19 and the non-zero-on-error exit code
already in place. Files run in argument order; `-` may appear among file
arguments to splice stdin into the sequence. A missing/unreadable file reports a
clear error and exits non-zero without running later files.

## Acceptance criteria

- [ ] `mgconsole run a.cypherl b.cypherl` runs both files in order through the
      serial path, streaming results in the resolved format.
- [ ] `mgconsole run -` reads stdin, identically to bare `… | mgconsole`.
- [ ] `-` may be mixed with file arguments and is spliced in at its position.
- [ ] `run` accepts the same `--import-mode` / `--output-format` / csv options as
      the top-level path; those top-level flags still work unchanged.
- [ ] A query failure exits non-zero (reusing the existing batch exit-code path);
      the failed query is named on stderr.
- [ ] A missing/unreadable file reports a clear error, exits non-zero, and does
      not run later files.

## Blocked by

- `.scratch/console-ergonomics/issues/19-tty-aware-default-output-format.md`
