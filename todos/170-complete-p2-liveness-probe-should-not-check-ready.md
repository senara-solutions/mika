---
status: complete
priority: p2
issue_id: "170"
tags: [code-review, architecture, operations]
---

# Fix Liveness Probe to Not Check Ready Flag

## Problem Statement
The `/livez` endpoint checks `state.ready.load()` and returns 503 when `ready` is false. During startup (before Postgres connection, migrations, webhook registration), liveness returns 503. If K8s `livenessProbe` fires before initialization completes, K8s will restart the pod — defeating the entire purpose of the liveness/readiness split.

A liveness probe should return 200 if the process is alive (can serve HTTP), regardless of readiness. Readiness gates traffic routing; liveness gates restart decisions.

## Findings
- **Architecture strategist**: Moderate risk — K8s `initialDelaySeconds` might mask this, but code-level behavior is non-standard
- **Performance oracle**: Prevents unnecessary pod restarts during slow migrations
- **Code simplicity reviewer**: The whole point of splitting /livez and /readyz is defeated if both check `ready`
- **4-agent consensus** on this finding

## Proposed Solutions

### Option A: Unconditionally return 200 (Recommended)
```rust
async fn handle_liveness() -> StatusCode {
    StatusCode::OK
}
```
The fact that the HTTP server is responding proves the process is alive.
- **Effort**: Trivial (2 min)
- **Risk**: None — standard K8s liveness pattern

### Option B: Use a separate liveness flag for deadlock detection
Add a heartbeat mechanism that sets a `last_alive` timestamp, and the liveness probe checks it.
- **Effort**: Medium
- **Risk**: Over-engineering for current scale

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs:402-408`

## Acceptance Criteria
- [ ] `/livez` returns 200 unconditionally when HTTP server is listening
- [ ] `/readyz` still checks `ready` flag and DB connectivity
- [ ] No restart loops during slow startups

## Work Log
- 2026-02-24: Created from code review of commit 9de9ba6

## Resources
- Commit: 9de9ba6
- K8s docs: liveness vs readiness probe semantics
