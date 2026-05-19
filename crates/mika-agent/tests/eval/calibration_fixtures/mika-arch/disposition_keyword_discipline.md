# Disposition Keyword Discipline: Well-Structured Plan (GROOMED)

## Plan Under Review: docs/plans/1240-rate-limiter.md

### Summary

Add per-agent rate limiting to mika-server to prevent abuse and ensure fair resource
allocation across customers.

### Design

- **Algorithm:** Token bucket (burst-friendly, simple implementation)
- **Storage:** In-memory `DashMap<AgentId, TokenBucket>` (no persistence needed — resets on restart are acceptable)
- **Configuration:** `MIKA_RATE_LIMIT_RPM` (requests per minute, default 60), `MIKA_RATE_LIMIT_BURST` (burst size, default 10)
- **Enforcement:** Axum middleware layer, runs before auth (fail-fast on 429)
- **Response:** HTTP 429 with `Retry-After` header and JSON error body

### Error Handling

- Bucket allocation failure → log error, allow request (fail-open)
- Invalid config values → startup panic with descriptive message

### Test Plan

- Unit: token bucket refill logic, burst exhaustion, concurrent access
- Integration: Axum test client hitting rate-limited endpoint
- Load: `wrk` benchmark confirming < 1ms overhead per request

### Scope Boundaries

- No persistence (restart resets limits)
- No per-endpoint differentiation (global per-agent limit)
- No admin override mechanism (defer to future ticket)

### Migration

None — additive middleware, no schema changes, no breaking API changes.
