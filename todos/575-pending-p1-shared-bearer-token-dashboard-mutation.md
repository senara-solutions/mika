---
status: pending
priority: p1
issue_id: "575"
tags: [code-review, security]
dependencies: []
---

# Shared Bearer Token Between Dashboard and Mutation Endpoints

## Problem Statement
The dashboard React app uses `VITE_MIKA_TOKEN` which is the same `MIKA_INTERNAL_TOKEN` used for gateway-to-agent authentication on `/message` and `/tasks/{id}/complete`. Anyone who opens browser DevTools can extract the token from compiled JavaScript or the Network tab, then use it to send messages as if they were the gateway or complete callback tasks with malicious payloads.

## Findings
- **Source:** Security Sentinel agent
- **Severity:** HIGH — full compromise of message ingestion and task completion
- **Location:** `dashboard/src/api/client.ts` line 2, `crates/mika-agent/src/server/mod.rs` lines 90-93
- The CORS layer is applied to the entire router (not just dashboard routes), meaning the browser-based dashboard origin can POST to mutation endpoints

## Proposed Solutions

### Option A: Separate read-only dashboard token
- Add `MIKA_DASHBOARD_TOKEN` env var accepted only on `/api/v1/*` routes
- Split auth middleware so mutation endpoints only accept `MIKA_INTERNAL_TOKEN`
- **Pros:** Simple, minimal code change
- **Cons:** Another env var to manage
- **Effort:** Small
- **Risk:** Low

### Option B: Session-based auth (login form + HttpOnly cookie)
- Add a login endpoint that validates credentials and sets an HttpOnly cookie
- Dashboard routes check the cookie, not a Bearer token
- **Pros:** Token never in client JS
- **Cons:** More complex, needs session management
- **Effort:** Large
- **Risk:** Medium

## Recommended Action
Option A — introduce a separate read-only token

## Technical Details
- **Affected files:** `crates/mika-agent/src/server/auth.rs`, `crates/mika-agent/src/server/mod.rs`, `dashboard/src/api/client.ts`

## Acceptance Criteria
- [ ] Dashboard endpoints accept `MIKA_DASHBOARD_TOKEN` (or either token)
- [ ] `/message` and `/tasks/{id}/complete` reject the dashboard token
- [ ] CORS layer scoped to dashboard routes only

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Security Sentinel found shared token issue |

## Resources
- PR #89: feat: Observability Dashboard (MVP)
