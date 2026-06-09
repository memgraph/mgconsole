# 10 — Auth: username/password + hidden password prompt

Status: ready-for-agent

## Parent

`.scratch/rust-console/PRD.md`

## What to build

Authenticated connections: pass username and password to the Session, and when a
username is given without a password, prompt for it interactively with terminal
echo disabled so it never lands in shell history or on screen. Integration
tested against a `memgraph/memgraph` container configured with credentials.

## Acceptance criteria

- [ ] Username + password are passed through to the Session and authenticate successfully
- [ ] A username with no password triggers a hidden (no-echo) prompt
- [ ] Declining/empty password at the prompt fails cleanly with a clear message
- [ ] Wrong credentials produce a clear authentication error, not a panic
- [ ] Integration test authenticates against a credentialed container

## Blocked by

- `.scratch/rust-console/issues/09-cli-flag-surface.md`
