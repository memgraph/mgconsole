# 25 — Non-interactive pipe + serial import + exit codes

Status: done

## Parent

`.scratch/rust-console/PRD.md`

## What to build

The non-interactive Frontend path: when stdin is not a TTY, read queries from
the stream and run them serially in input order (the default Import mode),
streaming output in the selected format. Exit codes reflect success/failure so
the tool is usable in CI and scripts. This is the serial-import tracer and the
foundation parser-mode (28) and parallel import (30) build on.

## Acceptance criteria

- [ ] A non-TTY stdin is detected and drives non-interactive execution
- [ ] Queries piped in are executed serially in order
- [ ] Output is produced in the selected format, streamed where applicable
- [ ] A failing query produces a non-zero exit code; a clean run exits zero
- [ ] An end-to-end test pipes a cypherl stream in and asserts effects + exit code

## Blocked by

- `.scratch/rust-console/issues/21-streaming-record-api.md`
- `.scratch/rust-console/issues/14-session-error-taxonomy-reconnect.md`
