# 21 — `-c "QUERY"` one-shot

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md` — implements **ADR 0014** (non-interactive
stdio conventions).

## What to build

Add `mgconsole -c "QUERY"` (`--command`) to run a single query and exit, matching
the `psql`/`cypher-shell` one-shot convention. It runs the given query through
the non-interactive serial path and applies the same conventions as the rest of
ADR 0014: the TTY-aware default output format from issue 19 (so `mgconsole -c '…'
| jq` yields `jsonl` while the same command at a terminal tabulates), the
stdout-data / stderr-chrome discipline, and the non-zero-on-error exit code. The
query string is assembled through the same `QueryAssembler` the piped path uses,
so a `-c` string containing several `;`-separated statements runs them in order.

`-c` is non-interactive and bypasses both interactive Frontends regardless of
whether stdin is a tty (you can `-c` from a terminal). Connection/auth/profile
resolution is unchanged.

## Acceptance criteria

- [ ] `mgconsole -c "RETURN 1"` runs the query, prints its result, and exits.
- [ ] Output format follows issue 19's TTY-aware default (`jsonl` when stdout is
      piped, `tabular` at a terminal), overridable by `--output-format`.
- [ ] A multi-statement `-c "Q1; Q2"` runs both in order.
- [ ] A query error exits non-zero with the failure on stderr; only result data
      goes to stdout.
- [ ] `-c` works from an interactive terminal (does not launch the REPL/workbench)
      and composes with connection flags/`--profile`.

## Blocked by

- `.scratch/console-ergonomics/issues/19-tty-aware-default-output-format.md`
