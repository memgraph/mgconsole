# 12 — Live schema completion source

Status: done

## Parent

`.scratch/tui-workbench/PRD.md`

## What to build

Complete against the names the database actually holds. On connect (while the
Session is idle), fetch the Schema — labels, relationship types, property keys —
via Memgraph's schema feature **when it is enabled**, and register it as a second
`CompletionSource` alongside the static vocabulary (the architecture's stated
extension point). When the feature is off or unavailable, degrade **silently** to
static-only completion and never run a graph scan to derive names. A manual
refresh re-fetches. Parsing the schema-feature result into a set of names is a
pure function.

The exact Memgraph introspection query and its enabling setting are a build-time
verification item (see the PRD); prefer the schema feature over any scan.

## Acceptance criteria

- [x] With Memgraph's schema feature enabled, labels / relationship types /
      property keys are fetched eagerly on connect and offered as completion
      candidates alongside keywords/functions. (`fetch_schema` spawned on connect;
      `set_schema` adds a `SchemaSource` to the completer; integration-verified.)
- [x] With the feature off/unavailable, completion degrades silently to
      static-only; no graph scan is issued. (a failed metadata query maps to
      `None`; the queries are the schema-metadata procedures, never a scan.)
- [x] A manual refresh re-fetches the Schema; the fetch never competes with an
      in-flight user query (one Session). (Ctrl-R → `Effect::FetchSchema`; the
      fetch serialises behind any query on the shared `Arc<Mutex<Session>>`.)
- [x] The result→names parser is a pure unit test; the enabled/disabled paths are
      covered against a live Memgraph (testcontainers). (`parse_schema` unit tests;
      `tests/schema_integration.rs` enabled + empty/unavailable paths.)

## Build-time verification (resolved)

The introspection queries are the schema-metadata procedures
`CALL schema.node_type_properties()` and `schema.rel_type_properties()`, enabled
by `--storage-enable-schema-metadata=true`, verified against Memgraph 3.10.1.
On 3.10.1 the procedures are present and return empty (not an error) without
data, so the silent-degrade path is: a procedure error (feature truly absent)
→ `None`, and an empty result → no names — static-only either way.

## Blocked by

- `.scratch/tui-workbench/issues/02-nonblocking-execution-spine.md`
- `.scratch/tui-workbench/issues/11-static-completion-popup.md`
