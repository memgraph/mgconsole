# 13 — Named queries: `:save` / `:saved` / `:load` / `:forget`

Status: done

## Parent

`.scratch/console-ergonomics/PRD.md`

## What to build

Add Named queries (`CONTEXT.md`): queries saved by name for later recall, in a
tool-managed `~/.mgconsole/queries.toml` (separate from the hand-edited
`config.toml`, so the tool rewriting it never clobbers user comments). A named
query stores **text only** — a reusable template that keeps its `$param`
placeholders and resolves them against the **current** `:param` store at run
time, not a frozen snapshot. Recall **loads into the input** (REPL line /
Workbench editor) for review and edit; it never auto-runs. Flat verbs in the
shared vocabulary: `:save <name> [query]` (the given or last query), `:saved`
(list), `:load <name>` (recall to input), `:forget <name>` (delete — a
deliberately non-generic verb so it never reads as deleting data).

## Acceptance criteria

- [ ] `:save <name> [query]` stores a text template in `~/.mgconsole/queries.toml`;
      with no query given it saves the last query.
- [ ] `:saved` lists saved names; `:forget <name>` removes one; both reflect the
      file, and rewriting the file preserves the others.
- [ ] `:load <name>` places the template into the input for review/edit and does
      not auto-run; submitting runs it normally.
- [ ] A loaded template's `$param` references resolve against the current
      `:param` store at run time (templates, not frozen values).
- [ ] The verbs behave identically in the REPL and Workbench; unknown names
      report a clear error.

## Blocked by

- `.scratch/console-ergonomics/issues/02-mgconsole-state-dir-toml-settings.md`
