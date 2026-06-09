# PRD: Rust Memgraph console

Status: ready-for-agent

## Problem Statement

People who work with Memgraph from the terminal today use `mgconsole`, a C++
command-line client. It does its job, but it is built as a single REPL: a C
toolchain and OpenSSL are required to build it, large result sets are buffered
entirely in memory before anything prints, a long-running query can only be
escaped by killing the whole process, and there is no machine-readable output
for scripting. It is also hard to grow — adding a terminal UI, query
cancellation, or live schema-aware completion would mean fighting the existing
shape rather than extending it.

We want a console that does everything `mgconsole` does today, builds as a
self-contained static binary with no C dependencies, and is shaped so that the
features people actually ask a graph console for — cancel an in-flight query,
stream huge results, drive it from scripts, and eventually a full-screen
terminal UI — are additive rather than rewrites.

## Solution

A Rust console for Memgraph, delivered as a `core` library plus an interactive
REPL frontend, with the seam in place for a terminal-UI frontend and a script
runner later.

From a user's perspective it behaves like `mgconsole`: connect to a Memgraph
server, type Cypher, see results rendered faithfully (nodes, relationships,
paths, maps, lists, temporal values, spatial points, enums), set query
parameters, and import/export via cypherl — including the fast batched-parallel
import mode. On top of that, result-oriented formats (csv, jsonl, cypherl)
stream with bounded memory instead of buffering, and the tool is built to host
cancellation, scripting, and a terminal UI as the roadmap progresses.

The transport is a pure-Rust Bolt stack (`bolt-client` + `bolt-proto`) so the
binary needs no C toolchain or OpenSSL; see ADR 0001. The architecture is an
async (tokio) Core behind a `block_on` boundary, split from its Frontends, with
streaming-first Records; see ADR 0002.

## User Stories

1. As a Memgraph user, I want to connect to a server by host and port, so that I
   can run queries against it.
2. As a Memgraph user, I want to provide a username and password, so that I can
   connect to a secured database.
3. As a Memgraph user, I want to be prompted for my password when I give a
   username without one, so that my password never appears in shell history.
4. As a Memgraph user, I want to connect over SSL/TLS, so that my session is
   encrypted.
5. As a Memgraph user, I want the console to automatically try to reconnect when
   the connection drops, so that a transient network blip does not end my
   session.
6. As a Memgraph user, I want to type a Cypher query and see its results, so
   that I can explore my data.
7. As a Memgraph user, I want to write a query across multiple lines ending in a
   semicolon, so that I can format complex queries readably.
8. As a Memgraph user, I want to put several queries on one line, so that I can
   run them in sequence quickly.
9. As a Memgraph user, I want every Memgraph Value rendered faithfully — nodes,
   relationships, paths, lists, maps, scalars, temporal values, spatial points,
   and enums — so that I can trust what I see.
10. As a Memgraph user, I want results as an aligned table by default, so that I
    can read modest result sets easily.
11. As a Memgraph user, I want the table to fit my terminal width, so that rows
    do not wrap unreadably.
12. As a Memgraph user, I want csv output, so that I can pipe results into other
    tools.
13. As a Memgraph user, I want to control the csv delimiter, escape character,
    and quoting, so that the output matches the consumer's expectations.
14. As a Memgraph user, I want jsonl output, so that I can pipe results into
    tools like jq.
15. As a Memgraph user, I want cypherl output, so that I can dump query results
    as replayable Cypher.
16. As a Memgraph user, I want csv, jsonl, and cypherl output to stream with
    bounded memory, so that I can export results far larger than RAM.
17. As a Memgraph user, I want a warning (and a pointer to a streaming format)
    when a tabular result exceeds a row cap, so that I do not accidentally
    buffer a huge result.
18. As a Memgraph user, I want to see how long each query took, so that I can
    gauge performance.
19. As a Memgraph user, I want a summary of how many rows were returned, so that
    I can confirm the result size at a glance.
20. As a Memgraph user, I want to see any notifications Memgraph attaches to a
    result, so that I learn about hints and warnings.
21. As a Memgraph user, I want execution stats and, optionally, verbose
    execution info (cost, parse, plan, execute times), so that I can analyse and
    tune queries.
22. As a Memgraph user, I want a clear error message when a query fails, without
    losing my session, so that I can fix and retry.
23. As a Memgraph user, I want command history persisted across sessions, so
    that I can recall earlier queries.
24. As a Memgraph user, I want to choose where history is stored, or disable it,
    so that I control what is written to disk.
25. As a Memgraph user, I want syntax highlighting as I type, so that I catch
    mistakes earlier.
26. As a Memgraph user, I want keyword and function autocompletion, so that I
    type queries faster.
27. As a Memgraph user, I want `:help`, so that I can discover what the console
    can do.
28. As a Memgraph user, I want `:quit` and Ctrl-D, so that I can leave the
    shell.
29. As a Memgraph user, I want `:docs`, so that I can jump to Memgraph
    documentation.
30. As a Memgraph user, I want to set a query parameter from a Cypher expression
    with `:param`, so that I can separate query data from query text.
31. As a Memgraph user, I want `:params` to list my parameters and `:params
    clear` to remove them, so that I can manage the parameters in play.
32. As a Memgraph user, I want my parameters applied to every query I run, so
    that I do not have to restate them.
33. As an operator, I want to export a whole database to a cypherl file, so that
    I can back it up or move it.
34. As an operator, I want to replay a cypherl file back into Memgraph, so that
    I can restore or seed a database.
35. As an operator, I want a serial import that runs queries in order, so that I
    have a predictable, safe default for loading data.
36. As an operator, I want a batched-parallel import mode, so that I can load
    large datasets much faster.
37. As an operator, I want to set the batch size and the number of workers for
    parallel import, so that I can tune throughput to my hardware and database
    mode.
38. As an operator, I want parallel import to respect vertices-first ordering,
    so that edges find their endpoints and the resulting graph is correct.
39. As an operator, I want parallel import to retry batches that hit
    serialization conflicts with backoff, so that transient conflicts do not
    fail my import.
40. As an operator, I want a parser-only mode that reports on queries without
    executing them, so that I can validate an import file before touching the
    database.
41. As a script author, I want to run the console non-interactively by piping
    queries into it, so that I can automate database work.
42. As a script author, I want a non-zero exit code when something fails, so
    that my CI catches errors.
43. As a packager, I want a single static binary with no C toolchain or OpenSSL
    dependency, so that distribution is trivial across Linux, macOS, and
    Windows.
44. As a user on a long-running query, I want to cancel it with Ctrl-C and keep
    my session, so that a mistake does not cost me my shell. (Roadmap — the
    architecture must permit it.)
45. As a contributor, I want the engine usable without the REPL, so that a
    terminal UI or script runner can be added as another Frontend over the same
    Core. (Roadmap — the seam must exist.)

## Implementation Decisions

- **Two crates, one seam.** A `core` library owns the Session, the Value model,
  Value rendering, and the import engine, with no dependency on any Frontend. A
  binary crate hosts the CLI and the interactive REPL Frontend. The terminal-UI
  Frontend and script runner are future crates/modules over the same Core. This
  seam is mandated by ADR 0002; frontend concerns (line editing, terminal
  drawing, prompts) must never leak into the Core.

- **Transport: pure-Rust Bolt.** `bolt-client` for connection/handshake,
  `bolt-proto` for PackStream Values, worked at the raw-Value layer (not a
  high-level driver). Mandated by ADR 0001. A verification spike precedes the
  main build: confirm every Memgraph Value type (enum, point, the full temporal
  family, node, relationship, path, nested list/map) decodes faithfully, and
  extend the codec for any Memgraph struct signature `bolt-proto` does not know.
  If the gap is larger than a handful of signatures, ADR 0001 is revisited
  (mgclient FFI is the named fallback).

- **Runtime.** One multi-threaded tokio runtime. The REPL and serial import are
  synchronous code that calls `block_on` at the Bolt boundary. Cancellation and
  parallel import promote to `select!` / tasks. Per ADR 0002.

- **Session interface.** The Core exposes a Session that takes a query plus
  parameters and returns a streaming sequence of Records (header + rows), along
  with timing, notifications, stats, and optional verbose execution info.
  Reconnect-with-retry lives behind this interface.

- **Records are streaming-first; rendering is format-dependent.** The Session
  yields Records as a stream. `csv` / `jsonl` / `cypherl` render row-by-row with
  bounded memory. `tabular` buffers (human-scale output) up to a row cap, past
  which it warns and points to a streaming format. The tabular path may use a
  buffer-all table renderer; the streaming paths use row-oriented writers. This
  asymmetry is deliberate (ADR 0002).

- **Query parameters.** `:param <name> <expr>` evaluates the Cypher expression
  server-side (with existing parameters in scope) and stores a copy of the
  resulting Value; the parameter set is passed to every subsequent query.
  `:params` lists, `:params clear` empties. This mirrors today's behaviour.

- **Import engine.** Serial mode runs queries in input order. Batched-parallel
  mode runs Batches concurrently over a small pool of Bolt connections (a
  Session is a single stream, so N workers need N connections), bounded by a
  worker count, preserving vertices-first ordering and retrying conflicted
  Batches with backoff. Parser mode inspects queries and reports without
  executing. Batch size and worker count are configurable.

- **Clause scanner.** A fast lexer over query text detects the clauses needed
  for vertices-first ordering (create / match / merge / index / etc.). It is a
  pure function from text to detected clauses, independent of any DB or session.

- **CLI surface.** Preserve today's flags as the baseline: host, port, username,
  password, ssl, output format, fit-to-screen, csv options, history path /
  no-history, verbose execution info, import mode, batch size, workers, parser
  stats. Add `jsonl` as an output format.

- **Line editing.** The REPL Frontend uses a modern line editor providing
  multiline editing, history, syntax highlighting, and a completer architecture
  that can later complete against live database schema.

- **Errors.** The Core distinguishes a recoverable query error (report, keep the
  session) from a fatal connection error (trigger reconnect). The binary maps
  these to user-facing messages and exit codes.

## Testing Decisions

A good test here asserts **external behaviour** — what a user or a caller of the
Core observes — never internal structure. Tests should survive a refactor that
keeps behaviour identical.

Seams, highest first:

- **Value rendering (pure functions) — primary seam.** Construct Values and
  assert the rendered text per format. No database required. Carry over
  `mgconsole`'s golden-file style from `tests/input_output/` (an input fixture
  paired with expected `tabular` / `csv` outputs), extended with `jsonl`. This
  is where the bulk of coverage lives: every Value type, escaping, quoting, csv
  options, fit-to-screen, and the tabular row-cap warning.

- **Clause scanner (pure unit).** Feed query text, assert detected clauses,
  with emphasis on the vertices-first ordering decisions and edge cases
  (comments, multiline, multiple queries per line).

- **Core Session API (integration, live Memgraph).** Drive the Session against a
  real Memgraph: connect, run with and without parameters, observe
  notifications and stats, exercise the query-error vs fatal-error distinction
  and reconnect. The ADR-0001 fidelity spike graduates into an integration test
  that returns every Value type from a live server.

- **Import engine (integration, live Memgraph).** Feed cypherl streams and
  assert serial ordering, batched-parallel correctness under vertices-first
  ordering, retry-on-conflict behaviour, and parser-mode reporting. Assert the
  streaming-vs-buffered output boundary (bounded memory for streaming formats).

- **End-to-end CLI (thin goldens).** A small number of smoke tests that pipe
  stdin and assert stdout and exit code, mirroring `mgconsole`'s
  `run-tests.sh`. Kept thin because the rendering and scanner seams already
  carry the detail.

Live-Memgraph integration is preferred over mocked Bolt for the Session and
import seams, because protocol fidelity against a real Memgraph is exactly the
confidence ADR 0001 calls for.

The live Memgraph for integration tests comes from the official
`memgraph/memgraph` Docker image, started on demand by the test harness — the
`testcontainers` crate spinning up the image per test run is preferred so that
`cargo test` is self-contained and CI needs only a Docker daemon (no manually
provisioned server, no checked-in service config). Tests that exercise
batched-parallel behaviour put the container into the analytical storage mode
where ordering and concurrency matter. The same image and harness back the
ADR-0001 fidelity spike.

## Out of Scope

- A terminal-UI (TUI) Frontend. The Core/Frontend seam must make it possible,
  but no TUI is built in this PRD.
- In-flight query cancellation (Ctrl-C → Bolt `RESET`). The runtime must permit
  it; implementing it is a later increment.
- Live schema-aware autocompletion (completing against labels/properties fetched
  from the server). The line editor must allow it; only static keyword/function
  completion ships here.
- A full script-runner Frontend with `--query` / run-a-file ergonomics and rich
  machine-readable modes beyond piping stdin. Basic non-interactive piping is in
  scope; a dedicated scripting Frontend is not.
- Cluster/routing awareness (coordinators, replicas, failover routing).
- Connection pooling beyond what batched-parallel import needs internally.
- Output to formats beyond tabular / csv / jsonl / cypherl (e.g. Parquet,
  GraphML).
- Saved/named queries, watch mode, explicit transaction control commands.

## Further Notes

- ADR 0001 (pure-Rust Bolt driver) and ADR 0002 (async Core with a
  Core/Frontend seam) are the binding architectural decisions; this PRD
  implements them and should not contradict them.
- The CONTEXT.md glossary is authoritative for vocabulary: Core, Frontend,
  Session, Value, Record, Result stream, Query parameter, Notification, Import
  mode (serial / batched-parallel / parser), cypherl, Batch, vertices-first
  ordering.
- The fidelity spike is the single gating risk and should be the first
  increment; every other increment assumes Values decode faithfully.
- Minor library picks deliberately left open: temporal crate (`time` vs
  `chrono`, decided by what `bolt-proto` exposes) and the connection-pool crate
  for parallel import (`deadpool` vs `bb8`).
