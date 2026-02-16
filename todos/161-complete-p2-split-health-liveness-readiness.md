---
status: complete
priority: p2
issue_id: "161"
tags: [code-review, performance, operations]
---

# Split Health Into Separate Liveness and Readiness Endpoints

## Problem Statement
The single `/health` endpoint runs a Postgres `SELECT 1` on every probe (routes.rs:346-356). Under load, health probes compete for the 10-connection pool. A false 503 from pool exhaustion triggers container restarts, amplifying the problem.

## Findings
- **Performance oracle**: CRITICAL — false-positive liveness failures under load cause pod restart cascades

## Proposed Solutions

### Option A: Separate /livez and /readyz (Recommended)
```rust
// /livez: process alive, no DB (for livenessProbe)
async fn handle_liveness(State(state): State<AppState>) -> StatusCode {
    if state.ready.load(Ordering::Acquire) { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE }
}
// /readyz: DB check (for readinessProbe)
async fn handle_readiness(State(state): State<AppState>) -> StatusCode {
    // Same as current /health
}
// Keep /health as alias for /readyz for backwards compat
```
- Effort: Small (15 min)
- Risk: Low — deployment manifests must be updated to use new endpoints

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs`

## Acceptance Criteria
- [ ] `/livez` returns 200 without DB query
- [ ] `/readyz` checks DB connectivity
- [ ] `/health` still works (alias)
- [ ] Documentation notes which to use for liveness vs readiness probes

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
