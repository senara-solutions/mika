---
title: "arch: GitHub identity separation — event ingestion vs action authorship"
type: arch
status: draft
date: 2026-04-11
---

# arch: GitHub identity separation — event ingestion vs action authorship

## Overview

Define a clean, multi-tenant-ready model for GitHub identity in the Mika self-dev
loop. Separates two orthogonal concerns — **event ingestion** (webhook delivery) and
**action authorship** (git operations, PR reviews, comments) — and maps each to the
right identity mechanism.

## Problem Statement

The current setup conflates two distinct concerns under "GitHub identity":

1. **Event ingestion** — GitHub App bots registered to receive webhooks and route events
   into the gateway.
2. **Action authorship** — the identity that authors commits, opens PRs, posts reviews
   and comments.

Two GitHub App bots were created (`mika-dev-bot[bot]`, `mika-qa-bot[bot]`) before
machine users existed. They were the original solution to identity separation. Now that
`mika-platform-dev` and `mika-platform-qa` PATs exist, the bots' action-authorship role
is redundant — but their event-ingestion role is not. PATs cannot receive webhooks.

The existing `MIKA_GITHUB_TOKEN` env var is also currently not injected into exec
environments (tracked in mika#515), causing all `gh` CLI calls to fall back to the host
shell's ambient `gh auth` — usually the developer's own account.

## Decision

### Two concerns, two mechanisms

| Concern | Mechanism | Identity |
|---|---|---|
| Event ingestion | GitHub App | One App (`mika-platform[bot]`) |
| Dev actions (commits, PRs, issue comments) | PAT | `mika-platform-dev` |
| QA actions (reviews, approvals, blocks) | PAT | `mika-platform-qa` |

The App becomes a **pure ingestion pipe** — it receives events and nothing else. All
action surface moves to machine user PATs injected as `GH_TOKEN` into exec environments.

### One GitHub App, not two

Two bots were never necessary for routing. The gateway's routing table (event type →
agent) is the routing mechanism, not GitHub App identity. A single App receives all
events; the gateway dispatches based on event type. Collapsing to one App removes a
redundant installation, one set of App credentials, and the conceptual confusion of
"which bot did this action."

The second App (`mika-qa-bot`) becomes dormant. Do not delete infrastructure — just
stop using it for new work.

### Config placement

| Config | Location | Rationale |
|---|---|---|
| `MIKA_GITHUB_APP_ID` | gateway `.env` | Only the gateway mints installation tokens |
| `MIKA_GITHUB_APP_PRIVATE_KEY` | gateway `.env` | Same — signs all installation tokens |
| `MIKA_GITHUB_APP_INSTALLATION_ID` | gateway `.env` (single-tenant) → tenant DB row (multi-tenant) | See migration path below |
| `MIKA_GITHUB_TOKEN` (dev) | mika-dev `.env` | Agent-scoped PAT, injected as `GH_TOKEN` |
| `MIKA_GITHUB_TOKEN` (qa) | mika-qa `.env` | Agent-scoped PAT, injected as `GH_TOKEN` |

Agents **never** hold App credentials. They only ever receive a `GH_TOKEN`.

## Multi-tenant migration path

`MIKA_GITHUB_APP_INSTALLATION_ID` as a flat env var is a deliberate single-tenant
shortcut for the self-dev loop. It is not the target shape.

In mika-cloud, each customer installs the App on their own repo and receives a unique
installation ID. The target model:

- `MIKA_GITHUB_APP_ID` + `MIKA_GITHUB_APP_PRIVATE_KEY` — platform-wide, stay in gateway
  env (Senara owns the App)
- `installation_id` — moves into the customer/tenant DB table, looked up at token-mint
  time per request

The token minting path changes from:

```
env var MIKA_GITHUB_APP_INSTALLATION_ID
  → mint token
  → inject GH_TOKEN
```

to:

```
inbound event carries customer context
  → gateway looks up installation_id from tenant record
  → mint scoped token
  → inject GH_TOKEN into routed request
```

No changes needed now. The flat env var is a conscious simplification. When mika-cloud
tenant onboarding is built, move `installation_id` into the tenant table and add the
lookup. App credentials stay exactly where they are.

## Identity matrix (final state)

| Identity | Type | Owns | Does NOT own |
|---|---|---|---|
| `mika-platform[bot]` | GitHub App | Webhook receipt | Any git/PR action |
| `mika-platform-dev` | Machine user PAT | Commits, PR creation, issue comments | Reviews, approvals |
| `mika-platform-qa` | Machine user PAT | PR reviews, approvals, block verdicts | PR authoring |

Self-approval is structurally impossible: `mika-platform-dev` opens the PR,
`mika-platform-qa` reviews it — different GitHub accounts.

## Implementation (mika#515)

Wire `MIKA_GITHUB_TOKEN` → `GH_TOKEN` in the exec handler so all `gh` CLI calls inside
skills use the agent's machine user identity.

**Files likely affected:**

- `crates/mika-agent/src/skills/exec_handler.rs` (or equivalent exec path) — inject
  `GH_TOKEN` from `MIKA_GITHUB_TOKEN` before spawning the skill process
- `crates/mika-agent/templates/skills/*/handlers/run.sh` — verify `GH_TOKEN` is picked
  up naturally; remove any `gh auth login` / `gh auth switch` calls if present

**Constraints:**

- Do not remove `MIKA_GITHUB_APP_*` config fields or the App token minting code — it
  handles webhook receipt and is a separate concern
- No new crates
- `cargo clippy` + `cargo test` after changes

## Acceptance Criteria

- [ ] `MIKA_GITHUB_TOKEN` is injected as `GH_TOKEN` into all exec-spawned skill processes
  for both mika-dev and mika-qa agents
- [ ] PR opened by mika-dev appears authored by `mika-platform-dev`
- [ ] PR review posted by mika-qa appears authored by `mika-platform-qa`
- [ ] No `gh auth login` or `gh auth switch` calls in any skill `run.sh`
- [ ] Gateway App credentials (`MIKA_GITHUB_APP_*`) unchanged and functional
- [ ] `cargo clippy` clean, `cargo test` passes

## Out of scope

- Deleting `mika-qa-bot` GitHub App (dormant, not deleted — useful if multi-tenant
  needs per-agent App identities later)
- Multi-tenant `installation_id` DB migration (tracked separately, no forcing function yet)
- Any change to webhook routing logic (routing table is correct as-is)

## Sources & References

- mika#515 — `MIKA_GITHUB_TOKEN` exec injection (the immediate implementation ticket)
- `MIKA_GITHUB_APP_*` env vars: `docs/configuration.md`, `CLAUDE.md`
- GitHub App token minting: `crates/mika-gateway/` webhook handler
- Exec handler env scrubbing pattern: `crates/mika-agent/src/skills/builtin_handlers.rs`
