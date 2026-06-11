# 04 — Read-only mode (Bolt READ access mode)

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add Read-only mode (`CONTEXT.md`): a session-wide safety guard that sets every
Transaction's Bolt access mode to READ, so **Memgraph itself rejects writes** —
the Clause scanner is never asked to police it. It is settable as a `--read-only`
CLI flag and via `:set readonly on`, and (once profiles exist, issue 03)
storable in a connection profile. The asymmetry is deliberate: it can be turned
**on** at runtime, but turned **off** only at connect time or via a profile —
`:set readonly off` at runtime is refused. A `[read-only]` marker shows in the
REPL prompt and the Workbench status bar whenever it is active.

## Acceptance criteria

- [x] With read-only active, a writing query is rejected by the server (Bolt READ
      access mode applied to the transaction), verified against a live database.
- [x] `--read-only` and `:set readonly on` both activate it; the Clause scanner
      is not involved in the decision.
- [x] `:set readonly off` at runtime is refused with a clear message explaining
      it can only be cleared at connect time or via a profile.
- [x] A `[read-only]` marker appears in the REPL prompt and Workbench status bar
      while active, and is absent otherwise.
- [x] Read-only applies to autocommit (each query wrapped in a read transaction);
      explicit `:begin` carries the mode in issue 05.

## Blocked by

- `.scratch/console-ergonomics/issues/01-set-mechanism-vertical-display.md`

## Comments

Implemented (AFK).

**Key finding (verified by probe against Memgraph 3.10.1):** Memgraph *ignores*
Bolt access mode on an auto-commit `RUN` but *enforces* it on an explicit
transaction ("Accessor type READ and query type WRITE are misaligned!"). So
read-only autocommit must wrap each query in `BEGIN {mode: r}` … `COMMIT`.

- **Core** (`core/src/session.rs`, `result.rs`): `ConnectOptions.read_only` +
  `Session::{set_read_only,is_read_only}`. When read-only, `run_once` opens a
  `BEGIN {mode: r}` transaction before `RUN`; the lazy `RecordStream` COMMITs it
  once drained/discarded (`commit_on_done`). A rejected write `RESET`s (rolls
  back) and surfaces as `Error::Query`, leaving the Session usable. Two live
  integration tests cover rejection and a large committed read.
- **CLI** (`cli/src/lib.rs`): `--read-only` flag; `resolve_connection` ORs flag
  with the profile's `readonly` (flag only ever turns it on; the profile, a
  connect-time source, can turn it off).
- **`:set readonly on/off`**: parsed via the shared `MetaCommand`; the REPL and
  Workbench special-case `readonly` — `on` applies to the Session, `off` is
  refused at runtime with a message pointing at connect-time/profile. `:set`
  listing shows the current state. New `parse_on_off`/`on_off` helpers.
- **Marker**: REPL prompt gains `[read-only]` (tracked live via a shared
  `AtomicBool` the runner flips); Workbench status bar shows `[read-only]`.
- **Asymmetry/off-at-runtime**: enforced in the Frontends, not the Core (the Core
  `set_read_only` allows both directions; the policy is a Frontend concern).
