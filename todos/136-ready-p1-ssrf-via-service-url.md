---
status: ready
priority: p1
issue_id: "136"
tags: [plan-review, security]
dependencies: []
---

# SSRF via service_url in gateway routing

## Problem Statement
The Phase 3 plan stores `service_url` as freeform TEXT in the customers table and uses it directly to forward messages to containers. An attacker who gains write access to the customers table (or exploits an admin API) could set `service_url` to an internal network address (e.g., `http://169.254.169.254/` for cloud metadata, or internal K8s services), causing the gateway to make requests to arbitrary internal endpoints.

**Why it matters:** This is a classic SSRF vulnerability. The gateway runs inside the K8s cluster and can reach internal services that are not exposed externally.

## Findings
- Source: Security Sentinel (C-1), Architecture Strategist
- Location: Plan Phase 3.3 (routing.rs) — `reqwest::Client::post(&customer.service_url)`
- The plan does not validate or restrict `service_url` values
- No allowlist or URL pattern enforcement is specified

## Proposed Solutions

### Option 1: Compute service_url from customer_id (Recommended)
Instead of storing freeform URLs, compute the container URL from the customer_id using a deterministic pattern:
```rust
fn container_url(customer_id: &Uuid) -> String {
    format!("http://mika-{}.mika-agents.svc.cluster.local:8080", customer_id)
}
```
- **Pros**: Eliminates SSRF entirely, no URL stored in DB, follows K8s service naming
- **Cons**: Less flexible if containers move to different hosts
- **Effort**: Small
- **Risk**: Low

### Option 2: URL allowlist validation
Validate service_url against a regex pattern or domain allowlist before use.
- **Pros**: Flexible, allows different container locations
- **Cons**: Allowlist must be maintained, regex bypasses are common
- **Effort**: Small
- **Risk**: Medium (bypasses possible)

## Technical Details
- **Affected files**: Plan section 3.3 (routing.rs), schema definition
- **Related Components**: Container forwarding, admin provisioning

## Acceptance Criteria
- [ ] service_url cannot point to arbitrary internal/external endpoints
- [ ] Gateway can only reach legitimate agent containers
- [ ] Validated in code review before merge

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Security Sentinel and Architecture Strategist both flagged SSRF risk in service_url handling
