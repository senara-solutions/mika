---
status: complete
priority: p2
issue_id: 370
tags: [code-review, publishing, crates-io, architecture, security]
dependencies: []
---

# Consider publish = false for mika-gateway

## Problem Statement

`mika-gateway` is a Telegram webhook router that requires Postgres, a Telegram bot token, a webhook secret, and a shared internal bearer token. It is deployed via Docker, not `cargo install`. Publishing it to crates.io creates confusion (users cannot use it without infrastructure) and unnecessarily exposes internal architecture details (bearer token model, /send endpoint, customer registry).

Note: `mika-common` and `mika-agent` MUST be published because `mika-ai` depends on them transitively. `mika-gateway` is independent — it only depends on `mika-common`.

## Findings

- Security sentinel: Publishing expands attack surface, advertises internal communication architecture
- Architecture strategist: Binary requires infrastructure that `cargo install` users won't have
- Simplicity reviewer: Adding crates.io metadata to a Docker-deployed binary is YAGNI

## Proposed Solutions

### Option 1: Add publish = false to mika-gateway
- Prevents accidental publishing
- Remove `keywords`, `categories`, `readme` (no longer needed)
- Keep `description` and `repository` (useful for `cargo metadata`)
- **Pros:** Explicit intent; no user confusion; smaller attack surface
- **Cons:** Can't install gateway via cargo (was this ever desired?)
- **Effort:** Small
- **Risk:** Low

### Option 2: Keep publishing all four crates
- Some users may want to build gateway from source without Docker
- Cargo provides a convenient distribution channel
- **Pros:** Maximum flexibility
- **Cons:** Users will be confused by non-functional `cargo install`
- **Effort:** None
- **Risk:** Low

## Recommended Action

(To be decided during triage)

## Technical Details

- **Affected files:** `crates/mika-gateway/Cargo.toml`

## Acceptance Criteria

- [ ] Explicit decision documented on whether mika-gateway should be on crates.io

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-01 | Created from code review of commit 2eca502 | Security, architecture, and simplicity reviewers all recommended publish = false |

## Resources

- Commit: 2eca502 "Prepare crates for publishing to crates.io"
