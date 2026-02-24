---
status: ready
priority: p3
issue_id: "164"
tags: [code-review, architecture]
---

# Extract Bearer Auth Into Shared Middleware

## Problem Statement
The gateway's `/send` endpoint has inline Bearer token validation (routes.rs:287-295), duplicating the pattern from `crates/mika-agent/src/server/auth.rs` which uses proper Axum middleware. Two separate auth code paths must be kept in sync.

## Findings
- **Architecture strategist**: Duplicated logic; recommends shared middleware in mika-common
- **Learnings researcher**: Phase 2 docs established middleware pattern as canonical

## Proposed Solutions

### Option A: Extract to gateway-local middleware (Recommended)
Create auth middleware in the gateway, apply as route_layer on /send. Matches agent pattern.
- Effort: Small (30 min)
- Risk: None

### Option B: Move to mika-common for sharing
- Effort: Medium — requires cross-crate changes
- Risk: Low

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs`
- **Reference**: `crates/mika-agent/src/server/auth.rs`

## Acceptance Criteria
- [ ] Bearer auth extracted to middleware function
- [ ] Applied as route_layer on /send
- [ ] Webhook route unaffected (uses different auth)

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
