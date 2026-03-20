---
status: pending
priority: p2
issue_id: 705
tags: [code-review, architecture]
dependencies: []
---

# Agent card security scheme doesn't match actual auth

## Problem Statement
The agent card in `a2a_card.rs` advertises `apiKey` security scheme with header name `x-api-key`, but the actual A2A route uses `require_internal_token` middleware checking `Authorization: Bearer <token>`. A remote client following the agent card would use the wrong auth header and get 401.

## Findings
- `crates/mika-agent/src/a2a_card.rs` lines 30-37: advertises `SecurityScheme::ApiKey { name: "x-api-key" }`
- `crates/mika-agent/src/server/mod.rs` lines 139-142: A2A route uses Bearer token auth
- The gateway proxy in `a2a_routes.rs` does accept `x-api-key` OR `Authorization: Bearer`, so the mismatch is only at the agent-server level

## Proposed Solutions
Change the agent card's security scheme to `Http { scheme: "bearer" }` to match the actual auth, or ensure both auth methods are accepted consistently.

## Acceptance Criteria
- [ ] Agent card security scheme matches actual authentication mechanism
