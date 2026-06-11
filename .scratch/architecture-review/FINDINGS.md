# Architecture review — findings (2026-06-11)

Status: done

Two rounds of `/improve-codebase-architecture` + `/grill-with-docs`. Round 1
(below) swept the Meta-command dispatch, the Core, and the CLI wiring. Round 2
(at the end) swept the Workbench reducer — the surface round 1 skipped.
`report.html` is round 1's visual; round 2's report stays in `/tmp` (the skill
writes it there). Both rounds worked each candidate against the **deletion
test** and the real call graph / field lifetimes.

## Round 1 — Meta-command dispatch, Core, CLI wiring

Method: three parallel `Explore` agents; candidates rendered to `report.html`;
then each worked against the deletion test and the real call graph.

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

---

## Round 2 — the Workbench reducer

Round 1 left the Workbench internals unexamined. Round 2 swept `update.rs` (4.7k
lines) and `state.rs` (55-field `WorkbenchState`) plus the draw edge and the
untouched Core (`transport`/`error`/`value`).

**Came back clean (no deepening):** the draw edge (draw-writes-state is a
documented one-way seam; the Effect interpreter is thin glue; the query-lifecycle
tasks are well-factored) and the untouched Core (`transport` is a real
Plain/TLS seam; `error` classifies via `is_retryable()`/`leaf()` without leaking
codes; `value` is a faithful model). All three candidates were in the *shape of
the reducer's state*, same theme: **make impossible states unrepresentable.**

### 1 — Fold the in-flight query into `RunState::Running` · DONE

The one in-flight query was 5 fields scattered down `WorkbenchState` (`run` +
`running_statement`/`started`/`tag`, plus `running_buffer`), kept consistent by
hand across ~6 lifecycle functions; nothing prevented `Idle` + a stale
`running_*`. Grilling sharpened the scope: only the **three per-statement facts**
(`statement`, `started`, `tag`) are `update.rs`-internal and always read behind
`is_current` — they fold cleanly. `running_buffer` is a *Vec index* read
unguarded by the `•` tab marker and shifted by `new_buffer`/`close_buffer`
(buffer-identity, a different concern); `pending` is the batch queue. Both stay
top-level.

Shipped: `RunState::Running { id, statement, started, tag }`. `RunState` loses
`Copy`; id-only callers go through `RunState::running_id()`. The "all cleared on
idle" invariant is now structural. Commit: "fold the in-flight query into the
RunState::Running variant".

### 2 — One `Modal` sum type for the six overlays · DONE

Verified a **reachable bug**, which upgraded this from "moves syntax, not
semantics" to justified: the "is a modal open" check was triplicated (key
cascade, mouse ignore-list, draw) and the mouse copy had drifted (omitted
`search`/`command_line`), so a mouse-click on a result cell *while in-result
search was open* opened the cell-detail overlay on top — two overlays at once.

Grilling decided the full enum (the user chose it over a cheap targeted fix even
after the ~150-site cost was surfaced): `modal: Option<Modal>` with six variants
(`help_scroll`/`detail_scroll` folded in); `drawer`/`watch` stay separate
(orthogonal). `search` became a hard keyboard-modal (its mouse-during-search was
incidental, not a feature). Dispatch via a `Copy` `ModalKind` tag; draw matches
`&state.modal`; mouse ignored iff `state.modal.is_some()` — fixes the bug
structurally. Read/write views (`search()`/`completion()`/`detail()`/…) keep the
call sites close to before. Commit: "one Modal sum type for the six
input-capturing overlays" (incl. a regression test).

### 3 — Drop the Buffer parking dance · DEFERRED

The active Buffer's 5 fields live both top-level and parked in `buffers[active]`;
`checkout`/`take_active`/`install_active` hand-swap them. Real fragility, but
**no active bug**, dormant (per-Buffer fields stable since issue 18), and it only
fixes half — `running_buffer` is a Vec index, so the `+1`/`-1` index-shift in
`new_buffer`/`close_buffer` is a *separate* concern (needs stable buffer IDs).

Recorded shape for when it bites: group the 5 active fields into one
`active_buffer: Buffer` field — kills the swap fragility with ~40 mechanical
renames (much funneled through `shown()`/`shown_mut()`), **not** the report's
full in-place `self.buffers[self.active]` access (which has a Vec-index borrow
problem: two `Index`/`IndexMut` calls on the same Vec conflict). Deferred.
