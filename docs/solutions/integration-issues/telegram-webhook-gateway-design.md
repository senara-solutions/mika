---
title: "Telegram Webhook Gateway Design — mika-gateway"
problem_type: integration-issues
modules: [mika-gateway, telegram-webhook-router, customer-pairing, outbound-relay]
tags: [telegram, webhook, gateway, axum, ssrf-prevention, constant-time-comparison, pairing-tokens, deduplication, secret-management, concurrency, rust]
severity: high
date_solved: 2026-02-24
related_issues:
  - "PR #6: feat(gateway): implement mika-gateway Telegram webhook router"
  - "todos/136-153: pre-implementation review findings"
  - "todos/154-167: post-implementation review findings"
related_docs:
  - docs/plans/2026-02-24-feat-mika-gateway-telegram-router-plan.md
  - docs/plans/2026-02-24-feat-platform-systems-gateway-provisioning-heartbeat-plan.md
  - docs/plans/2026-02-24-feat-phase2-container-http-server-plan.md
  - docs/solutions/architecture-decisions/phase2-axum-http-server-architecture.md
  - docs/brainstorms/2026-02-24-platform-systems-brainstorm.md
---

# Telegram Webhook Gateway Design — mika-gateway

## Problem Statement

Mika containers (per-customer agent instances on Kubernetes) can receive HTTP requests via `/message` and send outbound messages via the gateway's `/send` endpoint. But no gateway existed to bridge Telegram users to their containers. Without it, there was no path from a Telegram user's message to their Mika agent.

**Core challenge:** Build a stateless, secure Axum HTTP service that:
1. Receives Telegram webhooks and routes text messages to the correct customer container
2. Handles customer pairing via cryptographically secure deep links
3. Relays outbound messages from containers back to Telegram
4. Persists deduplication state for multi-replica correctness

## Architecture

```
Telegram ──webhook──► ┌──────────────┐    ┌──────────────┐
                      │ mika-gateway │───►│  Postgres     │
                      │ (Axum)       │    │  (customers)  │
                      └──────┬───────┘    └──────────────┘
                             │
            POST /message    │    POST /send (callback)
            (computed URL)   │         ▲
                             ▼
                 ┌─────────────────────┐
                 │ mika-{customer_id}  │
                 │ Axum + agent loop   │
                 │ SQLite (PVC)        │
                 └─────────────────────┘
```

| Endpoint | Method | Auth | Body Limit | Purpose |
|----------|--------|------|-----------|---------|
| `POST /webhook/telegram` | POST | X-Telegram-Bot-Api-Secret-Token (constant-time) | 64 KB | Telegram webhook delivery |
| `POST /send` | POST | Bearer MIKA_INTERNAL_TOKEN (constant-time) | 256 KB | Container outbound messages |
| `GET /health` | GET | None | — | K8s liveness/readiness probe |

## Solution: Key Design Decisions

### 1. Computed Container URLs (SSRF Prevention)

**Problem:** Original design stored `service_url` in the database, allowing potential user-controlled URLs for message forwarding.

**Solution:** Compute URL deterministically from customer_id using K8s service naming:
```rust
fn container_url(customer_id: &Uuid) -> String {
    format!("http://mika-{customer_id}.mika-agents.svc.cluster.local:8080")
}
```

**Why it works:** No user input involved. URL is deterministic and verifiable. Eliminates SSRF entirely.

### 2. Cryptographic Pairing Tokens (Not UUIDs)

**Problem:** Using customer UUID in deep link (`/start <uuid>`) allows attackers to enumerate valid pairing links.

**Solution:** Generate 32 random bytes, hex-encode to 64-char string, store with 24h expiry. Token cleared after single successful use.

**Pairing flow (atomic single UPDATE):**
```sql
UPDATE customers
SET telegram_chat_id = $1, paired_at = now(), status = 'active',
    pairing_token = NULL, pairing_expires_at = NULL
WHERE pairing_token = $2
  AND telegram_chat_id IS NULL
  AND status = 'provisioned'
  AND (pairing_expires_at IS NULL OR pairing_expires_at > now())
```

Six conditions checked atomically. Generic error "Invalid or expired invite link" for all failure cases — no information leakage about whether token existed, was used, or expired.

### 3. Persistent Dedup via `last_update_id`

**Problem:** In-memory HashMap for tracking processed update_ids is lost on restart and broken for multi-replica deployments.

**Solution:** Store `last_update_id` in Postgres per customer. After successful forward, UPDATE last_update_id. On each webhook, check: `if update_id <= last_update_id { drop }`.

**Post-review improvement:** Make dedup atomic with a conditional UPDATE:
```sql
UPDATE customers SET last_update_id = $1 WHERE id = $2 AND last_update_id < $1
```
This eliminates the read-then-check race condition under concurrent webhooks.

### 4. Async Webhook Processing (Fire-and-Forget)

**Problem:** Telegram webhook timeout is 60 seconds. Synchronous error replies risk timeout and retry.

**Solution:** Return 200 to Telegram immediately. Wrap all downstream work in `tokio::spawn`:
```rust
tokio::spawn(async move {
    match parsed {
        ParsedMessage::Start { .. } => handle_pairing(&s, ...).await,
        ParsedMessage::Text { .. } => handle_text_message(&s, ...).await,
        ParsedMessage::Unsupported { .. } => { /* async "text only" reply */ },
        ParsedMessage::NoMessage => { /* ignore */ },
    }
});
StatusCode::OK
```

**Post-review improvement:** Add `tokio::sync::Semaphore` to bound concurrent spawned tasks, preventing resource exhaustion under burst traffic.

### 5. Typed TelegramApiError Enum

Follows the existing `ClaudeApiError` pattern from `mika-common`:
```rust
pub enum TelegramApiError {
    RateLimited { retry_after: Option<u64> },
    BotBlocked,
    BadRequest { message: String },
    Unauthorized,
    Other { status: u16, body: String },
    Network(#[from] reqwest::Error),
}
```

Enables targeted retry logic and consistent error handling across the codebase.

### 6. No Auto-Suspension on 403

**Problem:** In-memory consecutive-403 counter could auto-suspend customers. Counter lost on restart; enables mass-suspension attack.

**Solution:** Return GONE (410) to container on 403, log for manual review. No database mutations. Eliminates attack vector while preserving audit trail.

### 7. Generic Error Messages (No State Leakage)

Three categories only:
| Scenario | Reply |
|----------|-------|
| Not paired | "Please pair your account first. Use your invite link to get started." |
| Transient error | "I'm having trouble right now. Please try again in a moment." |
| Suspended | Silent drop + log |

## File Structure

Consolidated from planned 7 files to 4 after review:

| File | Lines | Purpose |
|------|-------|---------|
| `crates/mika-gateway/src/main.rs` | 94 | Startup, PG connect, migrations, setWebhook, graceful shutdown |
| `crates/mika-gateway/src/routes.rs` | 458 | All handlers, AppState, auth, helpers, DB types |
| `crates/mika-gateway/src/telegram.rs` | 342 | TelegramApiError, Update parsing, sendMessage/setWebhook |
| `crates/mika-gateway/src/settings.rs` | 97 | Config-rs with MIKA_ prefix, redacted Debug |
| `migrations/001_customers.sql` | 32 | Postgres schema with CHECK constraints |

## Post-Implementation Review Findings

A multi-agent code review identified 14 additional issues (6 P1, 4 P2, 4 P3). The most important patterns:

### Race Conditions in Read-Then-Check

The dedup logic separated SELECT from UPDATE — a race condition under concurrent webhooks. **Fix:** Use conditional `UPDATE ... WHERE last_update_id < $1` (single atomic statement).

**Lesson:** Any time you read a value then conditionally write, ask: "Can this be one atomic statement?" In SQL, conditional UPDATE/DELETE eliminates the race.

### Unbounded Task Spawning

`tokio::spawn` without concurrency limit means burst traffic can exhaust memory. **Fix:** `tokio::sync::Semaphore` to cap in-flight tasks. Return 503 if semaphore full.

**Lesson:** Every `tokio::spawn` in a request handler needs a concurrency budget. Provide a `BoundedTaskSpawner` wrapper in shared code.

### Missing Input Validation Before DB

Pairing token not validated as 64-char hex before hitting Postgres. **Fix:** Validate format at handler boundary. Use newtype pattern (`struct PairingToken(String)`) with validation in constructor.

**Lesson:** All external input must be validated at HTTP handler boundary before any business logic or DB access. Use strong types to make validity explicit.

### Default HTTP Client (No Timeouts)

`reqwest::Client::new()` has no connect/pool/request timeouts, causing indefinite hangs. **Fix:** Configure via `ClientBuilder` with explicit timeouts.

**Lesson:** Every outbound HTTP client must specify `connect_timeout`, `request_timeout`, and `pool_idle_timeout`. Provide a factory function in shared code.

### Nullable Expiration Allowing Immortal Records

`pairing_expires_at` nullable in schema allows tokens that never expire. **Fix:** `NOT NULL` constraint or `CHECK`.

**Lesson:** For every time-bounded resource (tokens, sessions, pairings), enforce expiration at the schema level with `NOT NULL` + `CHECK`.

### Inconsistent Secret Types

`webhook_secret` was `String` while `bot_token` was `SecretString`. **Fix:** All `*_secret`, `*_token`, `*_key` fields must use `SecretString`.

**Lesson:** Establish a rule in CLAUDE.md: all sensitive fields use `secrecy::SecretString`. No exceptions.

## Prevention Strategies

These patterns apply broadly to future Rust/Axum services:

| Pattern | Prevention | Testing |
|---------|-----------|---------|
| Custom crypto | Use `subtle`, `secrecy`, `zeroize` — no custom implementations | Timing analysis tools |
| Non-atomic read-write | Single atomic SQL statement | Concurrent load tests |
| Unbounded task spawning | `Semaphore` wrapper, export metrics | Burst/chaos tests |
| Missing input validation | Newtype pattern, validate at boundary | Property tests with invalid formats |
| Nullable TTLs | `NOT NULL` + `CHECK` constraints | Schema violation tests |
| Default HTTP timeouts | Factory function with explicit timeouts | Slow upstream simulation |
| Plain String secrets | `SecretString` for all sensitive fields | Heap dump analysis |
| Dead code accumulation | `cargo clippy` in CI, coverage reports | Zero-coverage detection |

## Conventions Established

These should be added to CLAUDE.md for future services:

```
- Crypto: Use subtle, zeroize, secrecy. No custom implementations.
- Secrets: All *_secret, *_token, *_key → SecretString or Secret<T>.
- Concurrency: Every tokio::spawn must have a Semaphore limit.
- Atomicity: Read-then-write logic must be single atomic DB statement.
- Validation: All external input validated at handler boundary before DB.
- Timeouts: HTTP clients must specify connect/request/pool timeouts.
- Schema: Time-bounded resources must have NOT NULL expiry + CHECK.
```

## Cross-References

- **Phase 2 auth patterns:** `crates/mika-agent/src/server/auth.rs` — Bearer token validation middleware (constant-time)
- **Phase 2 architecture doc:** `docs/solutions/architecture-decisions/phase2-axum-http-server-architecture.md` — Axum AppState, middleware, graceful shutdown
- **Outbound messaging:** `crates/mika-agent/src/messaging.rs` — GatewayMessageSender (container side)
- **Brainstorm:** `docs/brainstorms/2026-02-24-platform-systems-brainstorm.md` — Original design decisions
- **Implementation plan:** `docs/plans/2026-02-24-feat-mika-gateway-telegram-router-plan.md` — Full spec with 32 review findings
