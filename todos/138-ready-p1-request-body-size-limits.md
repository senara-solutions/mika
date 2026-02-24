---
status: ready
priority: p1
issue_id: "138"
tags: [plan-review, security]
dependencies: []
---

# Missing request body size limits on internet-facing endpoints

## Problem Statement
The Phase 3 plan does not specify `RequestBodyLimitLayer` or any body size limits on the Telegram webhook endpoint, which is internet-facing. An attacker could send arbitrarily large POST bodies to exhaust gateway memory or cause OOM kills. The existing Phase 2 container server sets 50,000 char max on text — the gateway needs similar protection at the HTTP layer.

**Why it matters:** The webhook endpoint is directly exposed to the internet (Telegram sends to it). Without size limits, it's trivially exploitable for resource exhaustion.

## Findings
- Source: Security Sentinel (C-3), Performance Oracle
- Location: Plan Phase 3.2 (routes.rs) — router construction
- No RequestBodyLimitLayer mentioned in the plan
- Telegram updates are typically < 10KB but malicious requests can be arbitrary
- The existing container server (Phase 2) does not set body limits either

## Proposed Solutions

### Option 1: Add RequestBodyLimitLayer to router (Recommended)
```rust
use tower_http::limit::RequestBodyLimitLayer;

Router::new()
    .route("/webhook/telegram", post(webhook_handler))
    .layer(RequestBodyLimitLayer::new(64 * 1024))  // 64KB for webhook
    .route("/send", post(send_handler))
    .layer(RequestBodyLimitLayer::new(256 * 1024))  // 256KB for /send
```
- **Pros**: Simple, Axum-native, prevents resource exhaustion
- **Cons**: None significant
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected files**: Plan Phase 3.2 (routes.rs)
- **Related Components**: All POST endpoints

## Acceptance Criteria
- [ ] Webhook endpoint has body size limit (64KB recommended)
- [ ] /send endpoint has body size limit (256KB recommended)
- [ ] Oversized requests return 413 Payload Too Large
- [ ] Limits documented in operational runbook

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Security Sentinel flagged missing body size limits on internet-facing webhook
