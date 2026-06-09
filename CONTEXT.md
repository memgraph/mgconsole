# Context

Glossary for the Rust Memgraph console. Terms only — no implementation detail.
When a term below conflicts with how a word is used in code or conversation,
the definition here wins until deliberately changed.

## Core / Frontend

- **Core** — the part of the tool that owns a database connection, turns a
  query plus parameters into a stream of results, and renders database values
  to text. It knows nothing about how a human drives it.
- **Frontend** — a way of driving the Core. The interactive REPL is the first
  Frontend; a full-screen terminal UI and a non-interactive script runner are
  other Frontends over the same Core.

## Session and results

- **Session** — a single live conversation with one Memgraph server over which
  queries run and results return, in order.
- **Value** — a single piece of data Memgraph returns: a node, relationship,
  path, list, map, scalar, temporal value, spatial point, or enum. The console's
  defining job is to render every Value faithfully.
- **Record** — one row of a result: an ordered set of Values, one per result
  column.
- **Record stream** — the Records of one query, consumed one at a time rather
  than held all at once. _Avoid_: result stream, records stream.
- **Query result** — the whole answer to one query: its header (column names),
  its Record stream, and the trailing summary (timing, notifications, stats) that
  the server sends only after the last Record. _Avoid_: result set, response.
- **Query parameter** — a named value supplied alongside a query and referenced
  inside it, so the query text and its data stay separate.
- **Notification** — advisory information Memgraph attaches to a result (e.g. a
  performance hint), distinct from the result data itself.
- **Enum** — a Memgraph Value naming one member of a user-declared enumerated
  type, written `Type::Member` (e.g. `Status::Active`). The console treats it as
  a first-class Value, not as the map Memgraph happens to transmit it as.

## Import / export

- **Import mode** — the discipline by which a batch of queries from a file is
  run:
  - **Serial** — run queries one after another in the given order.
  - **Batched-parallel** — run queries concurrently in batches for speed,
    accepting that correctness now depends on input ordering and on retrying
    conflicts.
  - **Parser** — inspect the queries and report on them without running any.
- **cypherl** — a plain-text file of Cypher queries, one logical query at a
  time, used to move a database's contents out of and back into Memgraph.
- **Batch** — a group of queries from an Import that are submitted together as
  one unit of concurrency and retry.
- **Vertices-first ordering** — the requirement that node-creating queries
  appear before the edge-creating queries that depend on them, so a
  Batched-parallel import produces a correct graph.
