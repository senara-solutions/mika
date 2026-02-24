---
status: ready
priority: p3
issue_id: "150"
tags: [plan-review, simplicity, security]
dependencies: []
---

# Simplify health endpoint — minimal public response

## Problem Statement
The plan's health endpoint returns uptime, version, and status information. For a public-facing gateway, the health endpoint should return minimal information. Detailed diagnostics (pool stats, uptime, version) should be behind authenticated /admin/* routes.

**Why it matters:** Public health endpoints that reveal version and uptime information help attackers fingerprint the service and plan attacks.

## Findings
- Source: Code Simplicity Reviewer (YAGNI), Security Sentinel (M-5), Performance Oracle
- Uptime tracking requires AtomicU64 or Instant — unnecessary complexity
- Version info aids fingerprinting
- K8s probes only need 200/503 status code

## Proposed Solutions

### Option 1: Minimal public health, rich admin status (Recommended)
Public `/health`: Return only 200 OK or 503 based on Postgres connectivity. No body or minimal `{"ok":true}`.
Admin `/admin/status` (behind auth): Pool stats, uptime, version, customer counts.
- **Pros**: Secure, simple, K8s-compatible
- **Cons**: Need admin auth for diagnostics
- **Effort**: Small
- **Risk**: Low

## Acceptance Criteria
- [ ] Public health endpoint returns only status code (200/503)
- [ ] No version, uptime, or internal details in public response
- [ ] Detailed status available behind admin auth (if admin API is added)

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Multiple agents flagged health endpoint information disclosure
