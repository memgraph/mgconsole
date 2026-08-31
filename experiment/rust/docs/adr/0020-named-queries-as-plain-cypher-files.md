# 20. Named queries are plain `.cypher` files in a directory, not a tool-managed `queries.toml`

Date: 2026-06-11

## Status

Accepted

Amends ADR 0012 (which placed named queries in a tool-managed `queries.toml`).
The single-state-directory decision of ADR 0012 is unchanged; only the
named-query *format* changes.

## Context

ADR 0012 settled where console state lives (`~/.mgconsole`) and listed
`queries.toml` as the tool-managed home for named queries — a single file the
console rewrites on every save and delete. That keeps rewrites atomic and the
directory tidy, but it makes the saved queries opaque: not greppable per query,
not git-diff-friendly, not hand-editable in the user's own editor, and owned by a
format only the tool understands.

Surveying peer database/stream TUIs (rainfrog, yozefu) surfaced the opposite
call: rainfrog stores its query favourites as plain `.sql` files on disk, one per
query, and its favourites pane simply lists the directory. The files are
greppable, git-versionable, and editable outside the tool.

This matters more for a graph console than for a generic one: a user who keeps a
folder of analysis queries (centrality, neighbourhood walks, schema probes) wants
them under version control and reachable from outside the console.

## Decision

Store each named query as one plain `.cypher` file in a tool-owned directory
under `~/.mgconsole` (e.g. `~/.mgconsole/queries/`), the file name carrying the
query name. The directory is the source of truth — there is no separate index to
keep in sync. Saving writes a file; deleting removes one; listing reads the
directory.

Because the files are plain Cypher, the same directory doubles as a `:source`
library: a named query *is* a loadable script, so a saved query and a sourced
file are one format, not two. The `MGCONSOLE_*` env overrides still apply.

This deliberately trades `queries.toml`'s atomic-rewrite tidiness for
transparency, and accepts the costs that come with a user-visible directory:
name collisions are possible, and a hand-edit can introduce a file the tool
must tolerate rather than reject. We treat the directory as authoritative and
parse leniently rather than guarding a private format.

## Consequences

- ADR 0012's `queries.toml` line is superseded: named queries become a
  `queries/` directory of `.cypher` files. `config.toml` (Settings and
  Connection profiles) is unaffected and remains the single hand-edited document.
- The CONTEXT.md definition of **Named query** is updated to match (plain
  `.cypher` files, directory as source of truth, doubles as a `:source`
  library).
- Named queries gain git history, grep, and external editing for free, and
  compose with `:source` without a second loader.
- The console must tolerate a hand-edited or partially-written directory:
  unparseable or oddly-named files are reported, not fatal. Save/delete operate
  on individual files, so a crash mid-write can leave at most one bad file, never
  a corrupt index.
- No migration shim: the tool is pre-release (per ADR 0012), so there is no
  `queries.toml` in the wild to convert.
