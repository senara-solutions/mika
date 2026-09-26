# Groomed With TBD: Plan With Unresolved Decisions (ITERATE)

## Plan Under Review: docs/plans/1244-webhook-auth.md

### Summary

Add authentication to inbound webhook endpoints so only verified senders can
deliver messages to the agent.

### Design

- **Auth method:** TBD — either HMAC-SHA256 signature verification or API key in header
- **Secret storage:** Webhook secrets stored in `~/.mika/.env` as `MIKA_WEBHOOK_SECRET`
- **Verification:** Middleware layer in Axum, runs before message parsing
- **Failure response:** HTTP 401 with JSON error body

### Implementation

1. Add `WebhookAuthLayer` Axum middleware
2. Parse the `X-Webhook-Signature` header (or TBD: `Authorization` header for API key approach)
3. Compare against computed HMAC or stored key
4. Wire into server startup before the `/message` route

### Port Configuration

- The webhook listener port is TBD — pick a port number that doesn't conflict
  with the existing mika-spirit (8080) or gateway (3001)

### Error Handling

- Missing secret env var → startup panic with descriptive message
- Invalid signature → HTTP 401, log `webhook_auth_failed` WARN
- Malformed header → HTTP 400

### Test Plan

- Unit: HMAC computation, header parsing
- Integration: Axum test client with valid/invalid signatures
- Edge case: empty body, missing header, expired timestamp

### Migration

None — additive middleware, no schema changes.
