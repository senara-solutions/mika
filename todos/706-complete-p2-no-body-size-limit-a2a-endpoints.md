---
status: pending
priority: p2
issue_id: 706
tags: [code-review, security]
dependencies: []
---

# No body size limit on A2A endpoints

## Problem Statement
The A2A endpoints in both the gateway (`a2a_routes.rs`) and agent server (`server/a2a.rs`) have no explicit body size limit. While Axum has a default 2MB limit on `Json` extraction, the existing `/message` endpoint explicitly sets 10MB. Large A2A payloads could consume memory during deserialization.

## Findings
- `crates/mika-gateway/src/a2a_routes.rs`: No `DefaultBodyLimit` layer on A2A proxy route
- `crates/mika-agent/src/server/a2a.rs`: No body limit on `handle_a2a_jsonrpc`
- Existing `/message` route has explicit `RequestBodyLimitLayer::new(10 * 1024 * 1024)`

## Proposed Solutions
Add an explicit `DefaultBodyLimit::max(2 * 1024 * 1024)` (or 5MB) to A2A routes in both gateway and agent server.

## Acceptance Criteria
- [ ] A2A routes have explicit body size limits
