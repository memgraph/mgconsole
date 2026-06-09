# 09 — CLI flag surface + validation

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The complete clap-based CLI surface for the binary, preserving `mgconsole`'s
baseline flags and adding `jsonl` as an output format. Includes the validation
rules (e.g. output-format is one of the allowed set; csv-delimiter is a single
character; csv-escapechar required when doublequote is off). Flag parsing and
validation are tested without a database.

Baseline flags: host, port, username, password, use-ssl, output-format,
fit-to-screen, csv-delimiter, csv-escapechar, csv-doublequote, history path,
no-history, verbose-execution-info, import-mode, batch-size, workers-number,
parser-stats. Add output-format `jsonl`.

## Acceptance criteria

- [ ] All baseline flags parse with sensible defaults matching today's tool
- [ ] `jsonl` is accepted as an output format alongside tabular/csv/cypherl
- [ ] Invalid output-format is rejected with a clear message
- [ ] csv-delimiter must be a single character; csv-escapechar is required when csv-doublequote is false
- [ ] `--help` and `--version` work
- [ ] Parsing/validation covered by pure tests

## Blocked by

- `.scratch/rust-console/issues/02-tracer-bullet-workspace-connect.md`
