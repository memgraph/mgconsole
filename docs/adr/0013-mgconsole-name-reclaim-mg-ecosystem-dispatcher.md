# 13. The tool reclaims the name `mgconsole`; `mg` is reserved for a future ecosystem dispatcher

Date: 2026-06-10

## Status

Accepted

## Context

This tool began as a port of the C++ `mgconsole` (a thin, readline-style Cypher
REPL) but was deliberately designed to outgrow it: ADR 0002 split a Frontend-
neutral Core from three Frontends (interactive REPL, full-screen Workbench,
non-interactive script runner), and ADR 0010 made the full-screen TUI workbench
the *default* interactive experience. The result is no longer a console clone —
it is a multi-Frontend Memgraph client whose three faces (batch / line / TUI)
are co-equal ways of driving one Core.

That forces the naming question the repo had been deferring. Two pressures:

- **What slot does this tool occupy?** ADR 0012 already chose `~/.mgconsole` and
  the `MGCONSOLE_*` environment overrides for its state, framed as a clean-break
  *successor* to the legacy tool's `~/.memgraph` — so the lineage is already half
  claimed in the plumbing, and the tool is pre-release with no real users.
- **There will be sibling tools.** The Memgraph ecosystem is expected to grow
  other command-line tools (import, admin, …). A name must not quietly annex the
  whole ecosystem namespace for this one tool.

The tempting short name `mg` fails the second pressure: taking it for *this* tool
conflates the ecosystem umbrella with one of its members, and would back us into
making this binary the host (and plugin loader) for all the others.

## Decision

- **This tool is named `mgconsole`.** It reclaims the name as its *destination*,
  positioned as the clean-break successor to the C++ tool (consistent with ADR
  0012). `mgconsole2` / `mgconsole-rs` exist only as temporary coexistence
  aliases while the legacy binary may still share a `$PATH`.

- **The name is deliberately Frontend-neutral.** It names the tool's *role* — the
  interactive-and-scriptable query client — not any one Frontend. Names that
  index a single Frontend (`mgstudio` / `mgbench` / `mgtui` for the Workbench,
  `mgrepl` for the line REPL) were rejected for undersells the other two and
  contradicting the Core/Frontend split of ADR 0002.

- **`mg` is reserved as a future ecosystem dispatcher, not this tool.** When a
  second tool exists, `mg` becomes a thin git-style multiplexer: `mg console`
  execs `mgconsole`, `mg import` execs `mgimport`, resolved via `$PATH` exactly
  as `git foo` execs `git-foo`. **No plugin architecture** — every tool is an
  independent, standalone binary that also runs on its own; `mg` is a ~50-line
  exec shim built only when it earns its keep. Bare `mg` is therefore *not*
  taken now and *not* this tool.

## Consequences

- The naming aligns with the state directory and environment overrides already
  accepted in ADR 0012; nothing in the plumbing needs to change.
- During transition two binaries named `mgconsole` (C++ and Rust) could share a
  `$PATH`; the `mgconsole2` / `mgconsole-rs` aliases bridge that gap until the
  C++ tool is retired, after which this tool holds the name outright.
- The ecosystem gets unified `mg <verb>` ergonomics *and* independently
  shippable tools, with no plugin host, dynamic loading, or cross-tool coupling
  — the very thing reserving `mg`-as-member would have forced.
- A future reader is warned off two moves: renaming this tool to a
  Frontend-specific name (it would mislabel two of its three faces), and building
  a plugin architecture to justify `mg` (the git `$PATH` convention makes one
  unnecessary).
