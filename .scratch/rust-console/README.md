# Rust Memgraph console

A Rust rewrite of `mgconsole` (the C++ Memgraph CLI), built as a reusable Core
library plus frontends so that cancellation, streaming, scripting, and a future
terminal UI are additive rather than rewrites.

- **PRD:** [`PRD.md`](./PRD.md)
- **Issues:** [`issues/`](./issues/) — 32 tracer-bullet slices, numbered in
  dependency order.
- **Glossary:** `/CONTEXT.md` (repo root) — authoritative vocabulary (Core,
  Frontend, Session, Value, Record, Result stream, Import mode, cypherl,
  vertices-first ordering, …).
- **Binding decisions:** `/docs/adr/0001-pure-rust-bolt-driver.md` and
  `/docs/adr/0002-async-core-frontend-architecture.md`.

## Architecture in one paragraph

A pure-Rust Bolt stack (`bolt-client` + `bolt-proto`) talks to Memgraph — no C
toolchain, no OpenSSL (ADR 0001). One tokio runtime, with `block_on` at the Bolt
boundary so the REPL reads as synchronous code; features that need it
(cancellation, parallel import) promote to `select!` / tasks (ADR 0002). A
`core` library owns the Session, Value model, rendering, and import engine with
no frontend assumptions; the interactive REPL is the first Frontend, with a TUI
and a script runner reachable over the same seam. Records stream; rendering is
format-dependent (csv/jsonl/cypherl stream with bounded memory, tabular buffers
with a row cap).

## Testing

- Pure-function seams (rendering per Value type, line parsing, clause scanner,
  the format writers) are tested with no database — golden files ported from
  `mgconsole`'s `tests/input_output/`.
- Integration seams (Session, import) run against a live `memgraph/memgraph`
  Docker image started on demand via the `testcontainers` crate, so `cargo test`
  needs only a Docker daemon.
- A thin layer of end-to-end CLI goldens (pipe stdin, assert stdout + exit code).

Built with TDD: each issue is sized to roughly one red-green-refactor cycle.

## Dependency graph

```
01 spike (HITL)
└─ 02 tracer + harness ──┬─ 03 render scalars ─┬─ 04 list/map ─┐
                         │                     ├─ 05 node/rel/path ─┼─ 08 tabular layout ─┐
                         │                     ├─ 06 temporal ──────┤                     │
                         │                     └─ 07 point/enum ────┘                     │
                         ├─ 09 CLI ─┬─ 10 auth                                            │
                         │          └─ 11 TLS                                             │
                         ├─ 12 params ───────────────────────────────────────┐           │
                         ├─ 13 notifications/stats                            │           │
                         ├─ 14 errors/reconnect ──────────────────┐          │           │
                         ├─ 15 line parsing ──────────────────────┼──────────┼─ 16 REPL loop ─┬─ 17 history
                         │                                        │          │                ├─ 18 highlight/complete
                         │                                        │          │                ├─ 19 help/docs
                         │                                        │          └─ 20 :param ◄────┤ (+12)
                         ├─ 21 streaming API ─┬─ 22 csv           │                            │
                         │                    ├─ 23 jsonl         │  (16 needs 08, 14, 15)─────┘
                         │                    └─ 24 cypherl ─┐    │
                         │                        14 ────────┼────┘
                         │                                   └─ 25 serial import ─┬─ 26 dump/export (+24)
                         │                                                        └─ 28 parser mode (+27)
                         ├─ 27 clause scanner
                         └─ 29 workers ───┐
                            25 ───────────┴─ 30 parallel exec ─ 31 vertices-first (+27) ─ 32 retry/backoff
```

`(+NN)` means an additional blocker beyond the line drawn.

## Suggested order

1. **01 — Bolt fidelity spike** (HITL). The one gate: proves the pure-Rust Bolt
   stack decodes every Memgraph Value type, or triggers the `mgclient` FFI
   fallback. Do this before anything else.
2. **02 — Tracer bullet.** Workspace, tokio, connect, run one query, plus the
   `testcontainers` harness (local `cargo test`, Docker daemon only). Unblocks
   almost everything.
3. Then run three TDD fronts in parallel:
   - **Rendering:** 03 → (04, 05, 06, 07 independent) → 08.
   - **Connection/CLI:** 09 → (10, 11).
   - **Pure seams:** 15 (line parsing), 21 (streaming API), 27 (clause scanner)
     — each unblocks a downstream cluster.
4. **16 — REPL loop** is the first big milestone: a usable interactive shell
   (needs 08 + 14 + 15). Then 17–20 add ergonomics and `:param`.
5. **Output + import:** 22–24 (writers) → 25 (serial import) → 26 (export) and
   28 (parser mode).
6. **Parallel import:** 29 (worker Sessions) + 30 (executor) → 31
   (vertices-first) → 32 (retry/backoff). The thickest correctness work; do it
   last.

The pure-function runs (03–08, 15, 22–24, 27) need no container, so they're the
fastest cycles for early momentum.

## Triage

All issues are `ready-for-agent` except **01** (`ready-for-human` — the ADR-0001
go/no-go decision). Status is recorded as the `Status:` line in each issue file.

## Out of scope (designed-for, not built here)

Terminal UI frontend, in-flight query cancellation, live schema-aware
autocomplete, a full script-runner frontend, cluster/routing awareness. The seam
makes them additive; see the PRD's "Out of Scope".
