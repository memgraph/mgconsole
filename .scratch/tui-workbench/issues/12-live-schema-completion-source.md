# 12 — Live schema completion source

Status: ready-for-agent

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

- [ ] With Memgraph's schema feature enabled, labels / relationship types /
      property keys are fetched eagerly on connect and offered as completion
      candidates alongside keywords/functions.
- [ ] With the feature off/unavailable, completion degrades silently to
      static-only; no graph scan is issued.
- [ ] A manual refresh re-fetches the Schema; the fetch never competes with an
      in-flight user query (one Session).
- [ ] The result→names parser is a pure unit test; the enabled/disabled paths are
      covered against a live Memgraph (testcontainers).

## Blocked by

- `.scratch/tui-workbench/issues/02-nonblocking-execution-spine.md`
- `.scratch/tui-workbench/issues/11-static-completion-popup.md`
