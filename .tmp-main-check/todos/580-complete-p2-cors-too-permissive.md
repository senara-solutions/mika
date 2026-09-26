---
status: pending
priority: p2
issue_id: "580"
tags: [code-review, security]
dependencies: []
---

# CORS Configuration Too Permissive and Scoped to Entire Router

## Problem Statement
The CORS layer uses `AllowMethods::any()` and `AllowHeaders::any()` and is applied to the entire router (including mutation endpoints `/message` and `/tasks/{id}/complete`). Combined with the shared token issue (#575), this enables browser-based attacks against mutation endpoints.

## Findings
- **Source:** Security Sentinel
- **Location:** `crates/mika-agent/src/server/mod.rs` lines 47-57, line 96

## Proposed Solutions
- Restrict `allow_methods` to `[GET, OPTIONS]`
- Restrict `allow_headers` to `[Authorization, Content-Type]`
- Scope CORS layer to only `/api/v1/*` dashboard routes via nested router

## Acceptance Criteria
- [ ] CORS only applies to dashboard routes
- [ ] Only GET and OPTIONS methods allowed via CORS
- [ ] Document MIKA_CORS_ORIGIN in .env.example

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Security Sentinel flagged permissive CORS |

## Resources
- PR #89
