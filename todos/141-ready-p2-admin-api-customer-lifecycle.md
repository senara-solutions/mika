---
status: ready
priority: p2
issue_id: "141"
tags: [plan-review, architecture, agent-native]
dependencies: []
---

# Add admin API for customer lifecycle management

## Problem Statement
The plan only exposes 3 endpoints (webhook, send, health). All customer lifecycle operations (create, list, update, suspend, reactivate, delete) require direct psql access. This blocks automated provisioning, testing, and operational tooling. The Agent-Native Reviewer flagged that only 3 out of 10 necessary operations are programmatic.

**Why it matters:** Without an admin API, every customer operation requires SSH + psql access. This is error-prone, not auditable, and blocks CI/CD integration testing.

## Findings
- Source: Agent-Native Reviewer (Critical), Architecture Strategist
- Missing operations: create customer, list customers, get customer, update customer, suspend, reactivate, delete, regenerate pairing token, get pairing status, bulk operations
- provision.sh script uses psql directly instead of API calls
- No re-activation path except manual psql UPDATE
- No way to programmatically test pairing flow

## Proposed Solutions

### Option 1: Add /admin/* routes behind separate auth (Recommended)
Add admin endpoints behind a separate MIKA_ADMIN_TOKEN:
- POST /admin/customers — create
- GET /admin/customers — list
- GET /admin/customers/:id — get
- PATCH /admin/customers/:id — update
- POST /admin/customers/:id/suspend — suspend
- POST /admin/customers/:id/reactivate — reactivate
- POST /admin/customers/:id/regenerate-pairing — new pairing token
- **Pros**: Full lifecycle management, testable, auditable
- **Cons**: More endpoints to implement and secure
- **Effort**: Medium
- **Risk**: Low

### Option 2: Defer to Phase 4
Keep psql-only for Phase 3, add admin API in Phase 4.
- **Pros**: Smaller Phase 3 scope
- **Cons**: Blocks testing automation, manual operations error-prone
- **Effort**: None now
- **Risk**: Medium (operational debt)

## Technical Details
- **Affected files**: Plan Phase 3.2 (routes.rs), new admin handlers
- **Related Components**: Provisioning, monitoring, CI/CD

## Acceptance Criteria
- [ ] Admin API supports full CRUD on customers
- [ ] Admin endpoints behind separate auth token
- [ ] provision.sh uses admin API instead of psql
- [ ] Integration tests can create/pair/message/cleanup programmatically

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Agent-Native Reviewer flagged missing programmatic customer management
