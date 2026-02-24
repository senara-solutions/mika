---
title: "Gateway Post-Fix Review: 10 Findings Resolution (TODOs #168-#177)"
date: "2026-02-24"
category: "code-review-workflow"
problem_type: "security-timing-leaks, operational-reliability, architecture-consistency"
severity: "critical-and-important"
components:
  - mika-gateway
  - mika-agent
  - mika-common
tags:
  - constant-time-comparison
  - timing-side-channels
  - webhook-status-codes
  - kubernetes-probes
  - connection-pooling
  - secrets-management
  - message-deduplication
  - database-constraints
  - serde-field-mismatch
  - capacity-planning
resolution_time: "single session, 3-phase parallel execution"
verified: true
test_coverage: "171 tests passing"
static_analysis: "cargo clippy clean"
commit: "05fef61"
---

# Gateway Post-Fix Review: 10 Findings Resolution

## Context

This documents the resolution of 10 code review findings (#168-#177) discovered during a multi-agent re-review of commit `9de9ba6`. That commit had resolved the original 32 TODOs (#136-#167) from the Phase 3 gateway implementation. The re-review caught second-order issues across security, reliability, architecture, observability, and UX.

**Resolution commit:** `05fef61`
**Previous round:** `9de9ba6` (32 TODOs resolved)

## Problem Symptom

After resolving 32 initial code review findings, a 6-agent parallel re-review identified 10 new issues:
- 2 P1 (critical security/reliability)
- 4 P2 (important architecture/operations)
- 4 P3 (nice-to-have quality/UX)

## Root Causes and Solutions

### #168: constant_time_eq Length Timing Leak (P1 Security)

**Root cause:** `subtle::ConstantTimeEq` for `[T]` short-circuits on length — returns `Choice::from(0)` immediately when slices differ in length. An attacker can determine the exact length of `webhook_secret` by measuring response time.

**Fix:** Enforce 64-char hex token format at startup. Length becomes public knowledge.

```rust
// crates/mika-gateway/src/settings.rs
fn validate_hex_token(token: &SecretString, name: &str) -> anyhow::Result<()> {
    let val = token.expose_secret();
    if val.len() != 64 || !val.bytes().all(|b| b.is_ascii_hexdigit()) {
        anyhow::bail!("{name} must be exactly 64 hex characters (32 bytes hex-encoded)");
    }
    Ok(())
}

// Applied to both internal_token and telegram_webhook_secret in load()
validate_hex_token(&settings.internal_token, "MIKA_INTERNAL_TOKEN")?;
validate_hex_token(&settings.telegram_webhook_secret, "MIKA_TELEGRAM_WEBHOOK_SECRET")?;
```

Same validation added in `crates/mika-common/src/config.rs` for the agent's optional `internal_token`.

### #169: Webhook Semaphore Silent Message Loss (P1 Reliability)

**Root cause:** Returning `StatusCode::OK` when semaphore exhausted tells Telegram "delivered successfully" — no retry, permanent message loss with no signal.

**Fix:** Return 503 so Telegram retries with exponential backoff (~24h).

```rust
// crates/mika-gateway/src/routes.rs
Err(_) => {
    warn!("webhook at capacity, shedding load");
    return StatusCode::SERVICE_UNAVAILABLE; // was: StatusCode::OK
}
```

### #170: Liveness Probe Checked Ready Flag (P2 Architecture)

**Root cause:** Both `/livez` and `/readyz` checked `state.ready`, defeating the liveness/readiness split. K8s would restart pods during slow migrations.

**Fix:** Liveness returns 200 unconditionally — HTTP response proves process is alive.

```rust
async fn handle_liveness() -> StatusCode {
    StatusCode::OK
}
```

### #171: Pool Exhaustion Under Load (P2 Performance)

**Root cause:** 30 semaphore permits x 2 queries/task = 60 peak acquisitions, but pool only had 10 connections.

**Fix:** Raised to 20 with capacity budget documentation.

### #172: SendPayload request_id Mismatch (P2 Observability)

**Root cause:** Agent sends `request_id` in JSON, gateway's `SendPayload` didn't include it. Serde silently ignores unknown fields.

**Fix:** Re-added `request_id: Option<String>` with `#[serde(default)]` and logging.

```rust
#[derive(serde::Deserialize)]
struct SendPayload {
    chat_id: i64,
    text: String,
    #[serde(default)]
    request_id: Option<String>,
}
```

### #173: SecretString Inconsistency (P2 Security)

**Root cause:** Gateway used `SecretString` for `internal_token` but agent stored as plain `String` — no zeroize-on-drop.

**Fix:** Upgraded to `SecretString` across 6 files in 3 crates. `expose_secret()` used only at auth comparison and HTTP header injection points.

### #174: Bare /start Wrong Message (P3 UX)

**Root cause:** Bare `/start` returned `ParsedMessage::Unsupported` triggering "I can only read text messages" — factually wrong since `/start` IS text.

**Fix:** Added `BareStart` variant with contextual welcome message.

```rust
// telegram.rs
BareStart { chat_id: i64 },

// routes.rs
ParsedMessage::BareStart { chat_id } => {
    let _ = s.telegram.send_message(chat_id,
        "Welcome! If you have an invite link, please use it to get started. \
         If you're already set up, just type a message.",
    ).await;
}
```

### #175: max_connections Missing from SetWebhookPayload (P3 Performance)

**Root cause:** Field removed during refactoring. Telegram defaults to 40 but semaphore is 30 — wastes ~10 JSON parses.

**Fix:** Re-added `max_connections: 30` to match semaphore.

### #176: Dedup-Before-Forward Message Loss (P3 Reliability)

**Root cause:** Atomic dedup claims `update_id` BEFORE forwarding. If forward fails (network timeout), message permanently lost.

**Fix:** CAS rollback on network failure — decrement `last_update_id` only if still at the value we set.

```rust
Err(e) => {
    // Reset dedup so Telegram retry can succeed (CAS prevents incorrect rollback)
    let _ = sqlx::query(
        "UPDATE customers SET last_update_id = last_update_id - 1 \
         WHERE id = $1 AND last_update_id = $2",
    )
    .bind(row.id)
    .bind(update_id)
    .execute(&state.pool)
    .await;
    warn!(error = %e, customer_id = %row.id, "container unreachable, dedup reset");
}
```

### #177: 23505 Catch Too Broad (P3 Quality)

**Root cause:** Postgres 23505 catch didn't check which constraint was violated. `pairing_token` collision would show wrong "already linked" message.

**Fix:** Check constraint name via `db_err.constraint()`.

```rust
let msg = if db_err.constraint().is_some_and(|c| c.contains("telegram_chat_id")) {
    "This Telegram account is already linked to another account."
} else {
    "Pairing failed. Please contact support."
};
```

## Resolution Strategy

3-phase parallel execution to maximize throughput while respecting file conflicts:

1. **Phase 1** — Independent files in parallel: #171 (main.rs pool), #175 (telegram.rs max_connections)
2. **Phase 2** — Cross-crate SecretString upgrade: #173 (6 files across agent + common crates)
3. **Phase 3** — Sequential routes.rs changes: #168, #169, #170, #172, #174, #176, #177 (dependency: #169 before #176)

## Prevention Strategies

### 1. Constant-Time Comparison
- Validate token format (fixed length, expected charset) at startup, not just at comparison time
- Document what `subtle::ct_eq` actually guarantees — constant-time for same-length slices only
- Consider HMAC-based comparison if variable-length inputs are unavoidable

### 2. Webhook Return Codes
- `200 OK` = delivered, no retry. `503` = retry with backoff. `429` = rate limited
- Never return 200 for load shedding — it means permanent loss with webhook-based brokers
- Log all shed messages with update_id for monitoring

### 3. K8s Probe Semantics
- **Liveness** = "is the process alive?" Return 200 unconditionally
- **Readiness** = "can this serve traffic?" Check dependencies (DB, migrations)
- Never check initialization state in liveness — it causes restart loops

### 4. Capacity Planning
- Map: semaphore_permits x queries_per_task = peak_pool_acquisitions
- Pool size should handle peak with headroom (30 permits x 2 queries = 60 peak; 20 pool)
- Document the math in comments next to both semaphore and pool config

### 5. Serde Field Mismatches
- Use `#[serde(deny_unknown_fields)]` where appropriate to catch sender/receiver mismatches
- When threading correlation IDs (request_id), verify both sender and receiver structs
- Test with extra fields to verify rejection behavior

### 6. Secret Handling Consistency
- `SecretString` for ALL secrets across ALL crates — no plain String for tokens
- `expose_secret()` only at: auth comparison, HTTP header injection
- Custom `Debug` impls that redact secret fields

### 7. Atomic Operations with Rollback
- Use CAS (WHERE clause with expected value) for rollback, not timestamps
- Only reset on network-level failures (Err), not HTTP error responses (container may have processed)
- Log rollbacks with customer_id + update_id for debugging

### 8. Database Error Specificity
- Always check `constraint()` name, not just error code
- Different constraints warrant different user-facing messages
- Log full database error internally, show friendly message to user

## Review Checklist

### Security
- [ ] All secrets use SecretString with zeroize-on-drop
- [ ] Token format validated at startup (fixed length, expected charset)
- [ ] Constant-time comparison tested with equal-length strings
- [ ] Error messages don't leak internal details

### Reliability
- [ ] Non-2xx returned on transient failure (503/429, not 200)
- [ ] Shed/dropped messages logged with context
- [ ] Atomic operations have CAS rollback on failure
- [ ] Connection pool sized for peak concurrent load

### Operations
- [ ] /livez returns 200 unconditionally
- [ ] /readyz checks dependencies
- [ ] Correlation IDs threaded end-to-end

### Data
- [ ] Serde structs match sender/receiver contracts
- [ ] Database error handlers check constraint names
- [ ] User messages are contextual per failure mode

## Files Modified

| File | Changes |
|------|---------|
| `crates/mika-gateway/src/routes.rs` | #169 (503), #170 (liveness), #172 (request_id), #174 (BareStart), #176 (dedup reset), #177 (constraint) |
| `crates/mika-gateway/src/telegram.rs` | #174 (BareStart variant), #175 (max_connections) |
| `crates/mika-gateway/src/settings.rs` | #168 (hex token validation) |
| `crates/mika-gateway/src/main.rs` | #171 (pool size 20) |
| `crates/mika-common/src/config.rs` | #168 (internal_token validation), #173 (SecretString) |
| `crates/mika-agent/src/server/state.rs` | #173 (SecretString) |
| `crates/mika-agent/src/messaging.rs` | #173 (SecretString + expose_secret) |
| `crates/mika-agent/src/server/auth.rs` | #173 (expose_secret) |
| `crates/mika-agent/src/server/mod.rs` | #173 (test helper) |

## Cross-References

- **Previous round (32 TODOs):** `docs/solutions/code-review-workflow/iterative-multi-agent-review-and-resolution-cycle.md`
- **Gateway design:** `docs/solutions/integration-issues/telegram-webhook-gateway-design.md`
- **Review methodology:** `docs/solutions/code-review-workflow/parallel-agent-code-review-methodology.md`
- **Resolution strategy:** `docs/solutions/code-review-workflow/parallel-agent-code-review-resolution.md`
- **Implementation plan:** `docs/plans/2026-02-24-feat-mika-gateway-telegram-router-plan.md`
- **Commit chain:** `3f324f2` (implementation) -> `9de9ba6` (32 fixes) -> `05fef61` (10 fixes)
