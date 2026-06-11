# Architecture review — findings (2026-06-11)

Status: done

A deepening review (`/improve-codebase-architecture`) followed by a grilling
pass (`/grill-with-docs`). Method: three parallel `Explore` agents swept the
meta-command dispatch, the Core, and the CLI wiring; their candidates were
rendered to an HTML report (`report.html`); then each candidate was worked
against the **deletion test** and the real call graph.

**Net finding: the codebase is well-factored.** Three of the four candidates did
not survive grilling — the automated pass over-identified duplication it could
not disambiguate without the call graph. The one real issue was a single drifted
operator string, fixed.

This file is the durable record so a future review does not re-suggest the
rejected candidates. The HTML report (`report.html`) is the visual companion.

## A — Collapse the Meta-command dispatch onto one decision

**Was:** Strong. **Verdict: demoted → thin dedupe. Shipped.**

The premise was that `MetaCommand` is parsed once but dispatched twice —
`dispatch_meta` (`repl.rs`, ~165 lines, synchronous) and `handle_meta`
(`workbench/update.rs`, ~258 lines, async/Effects) — so the meaning of each
`:`-command is written twice.

On close reading the duplication is mostly **essential per-Frontend divergence**,
not two copies of one decision:

- A shared `CommandIntent` enum is a near-1:1 relabel of `MetaCommand`. It removes
  neither dispatcher's `match`; its only load-bearing content is the policy it
  carries. Strip the policy out and it is a pass-through — delete it and nothing
  concentrates. It fails the deletion test.
- The sync-vs-async execution split is **mandated by ADR 0010**: the non-blocking
  Workbench emits Effects, while `QueryRunner` is synchronous. The two execution
  machines are required to differ; they cannot move behind a shared seam.
- `:watch` (blocking loop vs tick-driven), `:source` (meta+Cypher vs Cypher-only
  per ADR 0017), and presentation (text vs overlay/drawer) are all deliberately
  different.

**What was real:** the read-only-off message had already *drifted* between the two
Frontends (the REPL named the `--read-only` / profile escape hatch; the Workbench
did not). Shipped: three `pub const` operator-message strings in `repl.rs`
(`READONLY_OFF_AT_RUNTIME`, `WATCH_REFUSED_IN_TX`, `TX_ABORTED_BY_CONNECT`),
referenced by both dispatchers. Refusal rules stay inline (trivially identical);
the two dispatchers stay separate by design. No intent layer, no new module, no
ADR. (Commit: "workbench/repl: canonicalize the shared Meta-command policy
messages".)

## B — Merge the result-layout split into one module

**Was:** Worth exploring. **Verdict: rejected.**

The proposal was to fold `core/src/tabular.rs` + `core/src/display.rs` into one
"layout" module (keeping the pure `core/src/render.rs`), because understanding how
a Display mode lays out a result bounces across three modules.

Rejected: the split tracks **three real consumer boundaries**, each with external
callers.

| Module | Job | External callers |
| --- | --- | --- |
| `render.rs` | Value → single-line cell text | format writers, REPL, workbench (×many) |
| `tabular.rs` | header+rows → aligned table | **`import.rs:170`**, `display.rs` |
| `display.rs` | Display-mode dispatch (tabular ⟷ vertical) | `main.rs`, workbench export |

The report assumed `render_table` was internal to `display.rs`, but `import.rs`
calls it **directly** — an import wants a plain table and has no notion of Display
mode (no `auto`, no `vertical`). Merging would force the import path to depend on
the Display-mode module just to get a table: *worse* coupling. The deletion test
runs backwards. The `render_records` interface is already deep; the
measure→decide "round-trip" is documented, deliberate reuse of the real width.

## C — Concentrate connection-target resolution

**Was:** Worth exploring. **Verdict: rejected.**

The proposal was to unify the two Connection-profile resolution paths
(`resolve_profile_connection` / `config.select` at startup vs
`resolve_connect_target` / `config.profiles.get` at `:connect`).

Rejected: they solve **different problems**.

- **Startup** is a per-field precedence merge (`explicit < CLI < profile`)
  producing a `Connection`. `config.select(name)` is *must-be-a-profile*: a typo'd
  `--profile` is fatal with a helpful "available profiles" error.
- **`:connect`** is profile-name-*or*-`host[:port]`, inheriting the current
  Session for a bare host, producing a `ConnectTarget`. `config.profiles.get` is
  *maybe-a-profile-else-a-host*: a non-match is parsed as an endpoint, and failures
  are reportable, not fatal.

The `select` vs `profiles.get` "inconsistency" encodes the different argument
grammars and error policies. The only literal overlap is a ~2-line credentials
wrap with different inputs (resolved `Connection` vs partial `Profile`). Shared
knowledge already lives in the `Profile` struct. A unifying resolver would *add*
complexity.

## D — Lift the Frontend drivers out of `main.rs`

**Was:** Speculative. **Verdict: skipped.**

Locality, not depth — and mis-aimed. The dispatch (`run_interactive`) is ~120
lines and belongs in `main`. The real bulk is ~500 lines of REPL *production
adapters* (`SessionRunner`, `RustylineSource`, `MgHelper`) sitting far from the
`QueryRunner` / `LineSource` seams they satisfy. Relocating them is a navigability
tidy with no interface or test change. The report's "`load_queries()` doubles FS
I/O" was wrong — it is two arms of a `match`; exactly one runs. Skipped as
non-architectural; revisit only if `main.rs`'s size actively bites.

## Reusable conclusion

The Explore-driven pass flagged duplication by name-and-shape similarity without
the call graph. Grounding each candidate in *who actually calls what* dissolved
three of four. Future reviews of this area should start from the call graph:
`import.rs` consumes `render_table`; the two connection resolvers produce
different types for different grammars; the two Meta-command dispatchers are
kept separate by ADR 0010.
