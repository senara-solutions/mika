---
title: GitHub Webhook Endpoint on mika-gateway
category: architecture-patterns
date: 2026-04-02
tags: [gateway, webhook, github, hmac, routing, axum]
related_issues: [382, 381]
modules: [mika-gateway]
---

# GitHub Webhook Endpoint on mika-gateway

## Problem

The mika-dev agent needed real-time push-based delivery of GitHub events (issues opened, PR reviews, CI failures) instead of relying on manual triggering or scheduled polling. This required adding a `POST /webhook/github` endpoint to the gateway that mirrors the existing Telegram webhook pattern but uses GitHub's HMAC-SHA256 signature validation.

## Root Cause / Design Challenge

GitHub App webhooks use HMAC-SHA256 body signing (not a simple header secret like Telegram). This means the handler must consume the raw body bytes for HMAC validation before JSON parsing. Axum's `Json<T>` extractor consumes the body, so the handler uses `Bytes` extraction followed by manual `serde_json::from_slice`. Additionally, GitHub webhook secrets are arbitrary strings (not 64-char hex like the gateway's other tokens), requiring a separate validation path.

## Solution

### New module: `crates/mika-gateway/src/github.rs`

**HMAC-SHA256 signature validation:**
```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub fn validate_signature(secret: &[u8], body: &[u8], signature_header: &str) -> bool {
    let hex_sig = match signature_header.strip_prefix("sha256=") {
        Some(s) => s,
        None => return false,
    };
    let Ok(expected) = hex::decode(hex_sig) else { return false };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else { return false };
    mac.update(body);
    let computed = mac.finalize().into_bytes();
    bool::from(computed.ct_eq(&expected))
}
```

**Handler pattern (raw Bytes for HMAC, then manual JSON parse):**
```rust
pub(crate) async fn handle_github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,  // Raw bytes for HMAC validation
) -> impl IntoResponse {
    // 1. Check config (optional — returns 404 if unconfigured)
    // 2. Validate X-Hub-Signature-256 via HMAC-SHA256
    // 3. Parse X-GitHub-Event header
    // 4. Handle ping event
    // 5. Dedup via X-GitHub-Delivery LRU cache
    // 6. serde_json::from_slice::<GitHubWebhookEvent>(&body)
    // 7. Bot self-event filter (match app_id + sender.type == "Bot")
    // 8. Route to agent via static map
    // 9. Async dispatch (tokio::spawn + semaphore permit)
    StatusCode::OK
}
```

**Event routing (static map):**
- `issues.opened/assigned` → mika-dev
- `issue_comment.created` → mika-dev
- `pull_request.opened/synchronize` → mika-qa
- `pull_request_review.submitted` → mika-dev
- `check_suite.completed(failure/timed_out)` → mika-dev

**Key design decisions:**
1. **Optional config** — `MIKA_GITHUB_WEBHOOK_SECRET` is `Option<SecretString>`. When absent, the route returns 404 (no breaking change for existing deployments).
2. **In-memory LRU cache** for delivery ID dedup (10k entries via `lru` crate), not Postgres. Simpler and sufficient since GitHub retries are rare.
3. **Single-tenant routing** — forwards to `agent_base_url` with `channel: "github"` and `chat_id: 0`. Multi-tenant routing deferred.
4. **Bot self-event filtering** — matches `installation.app_id` against `MIKA_GITHUB_APP_ID` config + `sender.type == "Bot"`, not `sender.login` suffix (more reliable, doesn't filter legitimate bots like Dependabot).
5. **Shared webhook semaphore** — reuses the existing 30-permit semaphore for unified backpressure.

### Dependencies added
- `hmac = "0.12"` — HMAC-SHA256 computation (RustCrypto)
- `lru = "0.12"` — delivery UUID dedup cache

### Config fields added to `GatewaySettings`
- `github_webhook_secret: Option<SecretString>` — `#[serde(default)]`
- `github_app_id: Option<u64>` — `#[serde(default)]`

Both redacted in `Debug` impls (AppState and GatewaySettings).

## Prevention / Best Practices

1. **Always validate HMAC over raw bytes before JSON parsing.** Using `Bytes` extractor (not `Json<T>`) is required for body-signed webhooks. This pattern applies to any webhook provider that signs the body (Stripe, Slack, etc.).

2. **Use `Option<SecretString>` for optional webhook secrets.** Making webhook secrets required would break existing deployments that don't use GitHub webhooks. The handler checks at the top and returns 404 when unconfigured.

3. **GitHub webhook secrets are arbitrary strings.** Do NOT apply hex-token validation (`validate_hex_token`) to GitHub secrets. They use a separate validation path.

4. **Bot self-event filtering should match `app_id`, not `sender.login`.** The `sender.login` suffix `[bot]` is shared by many GitHub Apps (Dependabot, Renovate). Matching the specific `installation.app_id` against the configured app ID is more precise.

5. **LRU cache poisoned-lock handling.** When using `std::sync::Mutex` for an LRU cache in an async context, always handle the poisoned case explicitly with a warning log. The fail-open pattern is correct for availability, but silent failure is not.

## Cross-References

- Phase 1 (prerequisite): #381 — GitHub App JWT signing module in `mika-common/src/github_app.rs`
- Telegram webhook pattern: `crates/mika-gateway/src/routes.rs` — the established pattern this follows
- A2A proxy pattern: `crates/mika-gateway/src/a2a_routes.rs` — similar auth-then-forward pattern
- Solution: [github-app-jwt-authentication-module.md](github-app-jwt-authentication-module.md)
- Solution: [multi-agent-telegram-delivery-and-reply-routing.md](../integration-issues/multi-agent-telegram-delivery-and-reply-routing.md)
