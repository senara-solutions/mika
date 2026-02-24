---
status: ready
priority: p2
issue_id: "147"
tags: [plan-review, architecture, reliability]
dependencies: []
---

# Add readiness gate pattern for Postgres connection

## Problem Statement
The plan specifies a health endpoint but doesn't mention a readiness gate for the Postgres connection pool. If the gateway starts accepting traffic before Postgres is connected, requests will fail. The existing Phase 2 container server doesn't have this pattern either, but the gateway is more critical since it's the single entry point for all customers.

**Why it matters:** In K8s, the readiness probe determines when a pod receives traffic. Without a Postgres readiness check, the gateway may receive webhooks before it can look up customers.

## Findings
- Source: Architecture Strategist (Medium), Performance Oracle
- Plan health endpoint returns 200 without checking Postgres connectivity
- K8s readiness probe should fail until Postgres pool is ready
- Existing container server uses a simple ready flag (AtomicBool)

## Proposed Solutions

### Option 1: AtomicBool ready flag + pool check (Recommended)
Match the container server pattern with an AtomicBool, but also verify Postgres pool:
```rust
async fn health(State(state): State<AppState>) -> impl IntoResponse {
    if !state.ready.load(Ordering::Relaxed) {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    // Quick pool check
    match state.pool.acquire().await {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
```
- **Pros**: K8s won't route traffic until gateway is truly ready
- **Cons**: One extra pool acquire per health check (negligible)
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected files**: Plan Phase 3.2 (routes.rs), health handler

## Acceptance Criteria
- [ ] Health endpoint checks Postgres connectivity
- [ ] K8s readiness probe fails if Postgres is unavailable
- [ ] Gateway doesn't accept traffic during pool initialization

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Architecture Strategist and Performance Oracle flagged missing readiness gate
