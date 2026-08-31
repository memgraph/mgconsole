# 9. Interactive input colouring is on by default

Date: 2026-06-10

## Status

Accepted

## Context

`mgconsole` ships input colouring off, and this port mirrored that: the
`--term-colors` flag defaulted off, with a test pinning the opt-in. But the
reason to highlight at all is to read a query's structure and catch a typo
*while typing*, before sending it. A typo-catcher you must discover a flag to
enable is not there the moment you fat-finger a keyword — the feature is dark for
everyone who does not already know it exists.

Highlighting only ever happens in the interactive REPL; a pipe takes the
non-interactive path, which never colours. So there is no risk of colour leaking
into machine-readable output regardless of the default.

## Decision

Interactive input colouring is **on by default**, controlled by a standard
`--color=auto|always|never` flag (replacing `--term-colors`), with `auto` — the
default — meaning on for an interactive terminal. The `NO_COLOR` environment
convention is honoured. Precedence is: an explicit `--color` beats `NO_COLOR`,
which beats the `auto` default.

This is a deliberate divergence from `mgconsole`'s default.

## Consequences

- The typo/readability signal is delivered to every interactive session without
  a flag, which is the point of building it.
- Drop-in `mgconsole` compatibility on this one default is given up on purpose;
  recording it here stops a future reader from "restoring" the old opt-in
  default and silently turning the feature off.
- `NO_COLOR` and `--color=never` remain the escape hatch for terminals or themes
  where the palette reads poorly; piped/non-interactive output is unaffected
  either way.
