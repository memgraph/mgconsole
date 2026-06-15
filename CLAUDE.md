# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`mgconsole` is a C++20 command-line client for the [Memgraph](https://memgraph.com) graph
database. It talks the Bolt protocol via the bundled `mgclient` library, reads Cypher from
either an interactive prompt (replxx) or stdin, and prints results as tabular / CSV / cypherl.

## Build

Dependencies are fetched and built from source via CMake `ExternalProject` (`gflags` pinned to
`70c01a6`, `mgclient` pinned to `v1.5.0`); `OpenSSL` and a C++20 compiler must be present. The
first configure/build is slow because of these external builds.

```bash
cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release .   # macOS: add -DOPENSSL_ROOT_DIR="$(brew --prefix openssl)"
cmake --build build
cmake --install build                                  # installs to /usr (Linux) or /usr/local (macOS) by default
```

The binary lands at `build/src/mgconsole`. `compile_commands.json` is emitted into `build/`.
`-Wall -Wextra -pedantic -Werror` is enabled — warnings break the build.

**Static / release build** (matches what ships): `./build-generic-linux.sh` builds inside the
`memgraph/mgbuild` Docker image with the Memgraph toolchain and `-DMGCONSOLE_STATIC_SSL=ON`,
producing `build/generic/mgconsole`.

## Test

Tests are wired into CTest and require a Memgraph instance (binary or Docker image) to run
against — they spin one up, exercise the client, then tear it down.

```bash
# Configure tests against a Docker Memgraph (no local binary needed):
cmake -B build -G Ninja -DMEMGRAPH_USE_DOCKER=ON -DMEMGRAPH_DOCKER_IMAGE=memgraph/memgraph:latest
cmake --build build
ctest --verbose --test-dir build           # runs all tests

ctest --test-dir build -R parameters-unit-test    # single unit test (no DB needed)
ctest --test-dir build -R mgconsole-test          # end-to-end I/O tests (plaintext)
ctest --test-dir build -R mgconsole-secure-test   # same, over SSL
```

To point at a local Memgraph binary instead of Docker, configure with
`-DMEMGRAPH_PATH=/path/to/memgraph` and leave `MEMGRAPH_USE_DOCKER=OFF` (default).

Two test kinds:
- **Unit** (`tests/unit/`): `parameters_test.cpp` links the standalone `params` library and runs
  without a database.
- **End-to-end** (`tests/input_output/`): driven by `run-tests.sh`. For every file in `input/`,
  it runs the client once per output format and diffs stdout against the matching golden file in
  `output_tabular/`, `output_csv/`, etc. **When you change output formatting or add an input
  case, regenerate/add the corresponding golden file in every `output_*` directory** — the test
  matrix is the cross-product of inputs × formats.

## Architecture

`main.cpp` parses gflags, sets up signal handlers, and dispatches to exactly one of four "modes"
based on whether stdin is a TTY and the `--import-mode` flag. Each mode lives in its own
translation unit under the `mode::` namespace and implements a `Run(...)` entry point:

- **`mode::interactive`** (`interactive.cpp`) — chosen when stdin is a TTY. The replxx REPL loop:
  reads a query, handles `:param`/`:params`/`:help`/`:quit` commands, executes via `mgclient`,
  prints results, manages history, and reconnects (3 retries) on fatal connection errors.
- **`mode::serial_import`** (`serial_import.cpp`) — default non-interactive (piped) path. Reads
  queries one at a time and executes them in order. This is the `DUMP DATABASE | mgconsole` path.
- **`mode::batch_import`** (`batch_import.cpp`) — `--import-mode=batched-parallel`. EXPERIMENTAL.
  Classifies each query (via `QueryInfo`) as pre/vertex/edge/post, groups vertex and edge queries
  into batches, and executes batches concurrently across a thread pool of `--workers-number`
  Bolt sessions, with exponential backoff + retry on failure. Vertices are flushed before edges
  because edges depend on existing vertices. Query classification is heuristic — that's why this
  mode is experimental.
- **`mode::parsing`** (`parsing.cpp`) — `--import-mode=parser`. Parses queries and prints
  `QueryInfo` stats without touching the database.

All four modes funnel through shared primitives in `src/utils/`, organized by namespace within
`utils.hpp`/`utils.cpp` (a large ~1300-line file):

- **`query::`** — `GetQuery()` reads and accumulates a complete (`;`-terminated, possibly
  multi-line) query from the input source, optionally producing `QueryInfo` (the has_create /
  has_match / has_merge / ... flags that drive batch classification). `ExecuteQuery()` /
  `ExecuteBatch()` run against an `mg_session`. `QueryResult` carries records, header, timing,
  notifications, and execution stats.
- **`console::`** — TTY detection, line reading, `Echo*` output helpers (failure/info/stats).
- **`format::`** — `CsvOptions` and `OutputOptions`; `Output()` renders a result set as tabular,
  CSV, or cypherl.
- **`utils::bolt`** (`bolt.hpp`/`bolt.cpp`) — `Config` struct and `MakeBoltSession()`, the single
  place sessions are created (direct or routing connection).

`src/parameters.{hpp,cpp}` is deliberately a **separate static library (`params`)** depending
only on `mgclient`, so the `:param` parsing/storage logic can be unit-tested in isolation. Don't
add heavier dependencies to it.

Concurrency support for batch mode is custom and lives in `utils/`: `thread_pool`, `future`
(promise/future with notification hooks), `notifier`, and `synchronized`. `mg_memory.hpp` wraps
raw `mgclient` C pointers in RAII unique-ptr types (`MgSessionPtr`, `MgValuePtr`, etc.) — use
these rather than managing `mg_*` lifetimes by hand.

## Conventions

- Every source file carries the GPLv3 license header — copy it onto new files.
- Formatting is enforced by `.clang-format` (Google base, 120 col). Run `clang-format` before
  committing.
- `MG_ASSERT` / `MG_FAIL` (`utils/assert.hpp`) are the assertion/abort macros.
- `date.hpp` is a large vendored third-party header (Howard Hinnant's date lib) — don't edit it.
