---
title: "feat: GitHub webhook endpoint on mika-gateway"
type: feat
status: active
date: 2026-04-02
issue: 382
---

# feat: GitHub webhook endpoint on mika-gateway

## Overview

Add a `POST /webhook/github` endpoint to `mika-gateway` for receiving GitHub App webhook events. This follows the existing Telegram webhook pattern: validate at the trust boundary, determine the target agent, forward to the agent container via the existing `/message` endpoint.

This is Phase 2a of the GitHub App integration (senara-solutions/mika-platform#3). Phase 1 (#381) added the `GitHubApp` JWT/token module to `mika-common`. This phase adds the inbound webhook receiver on the gateway.

## Problem Statement / Motivation

Mika's autonomous development loop (mika-dev agent) currently relies on manual triggering or scheduled polling to discover GitHub events (new issues, PR reviews, CI failures). A webhook endpoint enables real-time, push-based delivery of GitHub events directly to agent containers, enabling:

- Immediate issue triage when issues are opened/assigned
- Automatic PR review when PRs are opened or updated
- CI failure notification when check suites fail
- PR review comment processing when reviews are submitted

## Proposed Solution

Add a new module `crates/mika-gateway/src/github.rs` containing:
1. GitHub webhook event type definitions (serde deserialize)
2. HMAC-SHA256 signature validation using the `hmac` crate
3. Event-to-agent routing logic with a static routing map
4. `POST /webhook/github` route registration in `build_router()`

Follow the existing gateway patterns:
- `AppState` extended with optional `github_webhook_secret`
- `GatewaySettings` extended with optional `MIKA_GITHUB_WEBHOOK_SECRET`
- Forward to containers via `POST {container_url}/message` with `internal_token` bearer auth
- Spawn async task for forwarding, return 200 OK immediately

## Technical Approach

### Architecture

```
GitHub App ──POST /webhook/github──> mika-gateway
                                        │
                                        ├── 1. HMAC-SHA256 signature validation (X-Hub-Signature-256)
                                        ├── 2. Idempotency check (X-GitHub-Delivery LRU cache)
                                        ├── 3. Bot self-event filtering (MIKA_GITHUB_APP_ID match)
                                        ├── 4. Event type + action routing (static map)
                                        ├── 5. Customer resolution (single-tenant: agent_base_url)
                                        └── 6. Async dispatch: POST {container_url}/message
                                                channel: "github", agent: "<routed-agent>"
```

### Design Decisions

**Customer resolution: single-tenant for Phase 2a.** All GitHub webhook events route to the container at `agent_base_url` (local dev) or a single customer's container. Multi-customer routing (GitHub installation_id -> customer_id mapping via Postgres) is deferred to a future phase. Rationale: the GitHub App is initially installed on one org (senara-solutions), so multi-tenant routing adds complexity without immediate value.

**Message payload: reuse existing `MessageRequest`.** Forward GitHub events as `MessageRequest { text: "<markdown summary>", chat_id: 0, channel: "github", agent: "<target>", ... }`. The agent container already handles `channel` as a string. Using `chat_id: 0` signals "no reply channel" -- the agent should use GitHub tools (`run_gh`) for responses, not `send_message`. The `MessageSender` on the agent side will be `None` for `chat_id: 0` requests (or the agent's system prompt/skill context handles `channel: "github"` appropriately -- this is an agent-side concern for a later phase).

**Bot self-event filtering: match `app.id` field against `MIKA_GITHUB_APP_ID`.** More reliable than checking `sender.login` suffix because: (a) not all events have `sender`, (b) other legitimate `[bot]` users (Dependabot, Renovate) should not be filtered. The `app.id` field is present on events triggered by GitHub App installations. Falls back to `sender.login` ends-with `[bot]` check when `app.id` is absent.

**Idempotency: in-memory LRU cache (not Postgres).** An `lru` crate LRU cache with 10,000 entry capacity is simpler, faster, and sufficient. GitHub retries are rare (exponential backoff), and the window for duplicates during gateway restarts is small. No Postgres migration needed. Cache wrapped in `tokio::sync::Mutex` on `AppState`.

**Secret optionality: `MIKA_GITHUB_WEBHOOK_SECRET` is optional.** When absent, the `/webhook/github` route is still registered but returns 404 (unconfigured). This avoids breaking existing deployments. GitHub webhook secrets are arbitrary strings (not 64-char hex), so they use a separate validation path from `validate_hex_token()`.

**Concurrency: share existing `webhook_semaphore`.** No need for a dedicated semaphore -- the shared semaphore (30 permits) provides unified backpressure across all webhook traffic.

### Implementation Phases

#### Phase 1: Core Module (`github.rs`)

**File: `crates/mika-gateway/src/github.rs`**

1. **Event types** -- Minimal serde structs for the 5 routed event types plus `ping`:

```rust
// crates/mika-gateway/src/github.rs

#[derive(Debug, Deserialize)]
pub struct GitHubWebhookEvent {
    pub action: Option<String>,
    pub sender: Option<GitHubUser>,
    /// Present on events triggered by a GitHub App installation
    pub installation: Option<GitHubInstallation>,
    // Event-specific payload (flattened or ignored for routing purposes)
}

#[derive(Debug, Deserialize)]
pub struct GitHubUser {
    pub login: String,
    #[serde(rename = "type")]
    pub user_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GitHubInstallation {
    pub id: u64,
    pub app_id: u64,
}

#[derive(Debug, Deserialize)]
pub struct CheckSuitePayload {
    pub action: Option<String>,
    pub check_suite: Option<CheckSuite>,
    pub sender: Option<GitHubUser>,
    pub installation: Option<GitHubInstallation>,
}

#[derive(Debug, Deserialize)]
pub struct CheckSuite {
    pub conclusion: Option<String>,
}
```

All structs use `#[serde(default)]` or `Option` for unknown/missing fields -- GitHub adds new fields frequently. No `deny_unknown_fields`.

2. **HMAC-SHA256 validation:**

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub fn validate_signature(secret: &[u8], body: &[u8], signature_header: &str) -> bool {
    let hex_sig = signature_header.strip_prefix("sha256=").unwrap_or("");
    let Ok(expected) = hex::decode(hex_sig) else { return false };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else { return false };
    mac.update(body);
    let computed = mac.finalize().into_bytes();
    bool::from(computed.ct_eq(&expected))
}
```

3. **Event routing map:**

```rust
pub fn route_event(event_type: &str, action: Option<&str>, check_conclusion: Option<&str>) -> Option<&'static str> {
    match (event_type, action) {
        ("ping", _) => None, // handled separately
        ("issues", Some("opened" | "assigned")) => Some("mika-dev"),
        ("issue_comment", Some("created")) => Some("mika-dev"),
        ("pull_request", Some("opened" | "synchronize")) => Some("mika-qa"),
        ("pull_request_review", Some("submitted")) => Some("mika-dev"),
        ("check_suite", Some("completed")) => {
            match check_conclusion {
                Some("failure" | "timed_out") => Some("mika-dev"),
                _ => None,
            }
        }
        _ => None, // unroutable -- silently drop
    }
}
```

4. **Bot self-event filtering:**

```rust
pub fn is_bot_self_event(event: &GitHubWebhookEvent, app_id: Option<u64>) -> bool {
    // Primary: match installation.app_id against configured MIKA_GITHUB_APP_ID
    if let (Some(installation), Some(configured_app_id)) = (&event.installation, app_id) {
        if installation.app_id == configured_app_id {
            // Check if the sender is the app's bot user
            if let Some(sender) = &event.sender {
                if sender.user_type.as_deref() == Some("Bot") {
                    return true;
                }
            }
        }
    }
    false
}
```

5. **Message text formatting:**

```rust
pub fn format_event_text(event_type: &str, body: &serde_json::Value) -> String {
    // Produce a markdown summary suitable for the agent's context
    // Include: event type, action, repo, title/body, URLs
}
```

#### Phase 2: Route Registration and Handler

**File: `crates/mika-gateway/src/routes.rs`**

1. Add route in `build_router()`:

```rust
.route(
    "/webhook/github",
    post(github::handle_github_webhook)
        .layer(RequestBodyLimitLayer::new(256 * 1024)),
)
```

2. Handler function in `github.rs`:

```rust
pub async fn handle_github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. Check if GitHub webhook is configured
    let Some(ref secret) = state.github_webhook_secret else {
        return StatusCode::NOT_FOUND;
    };

    // 2. Validate X-Hub-Signature-256
    let sig = headers.get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok());
    let Some(sig) = sig else {
        return StatusCode::UNAUTHORIZED;
    };
    if !validate_signature(secret.expose_secret().as_bytes(), &body, sig) {
        warn!("GitHub webhook signature validation failed");
        return StatusCode::UNAUTHORIZED;
    }

    // 3. Parse X-GitHub-Event header
    let event_type = headers.get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    // 4. Handle ping
    if event_type == "ping" {
        info!("GitHub webhook ping received");
        return StatusCode::OK;
    }

    // 5. Idempotency via X-GitHub-Delivery
    let delivery_id = headers.get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !delivery_id.is_empty() {
        if let Ok(mut cache) = state.github_delivery_cache.lock() {
            if cache.put(delivery_id.to_string(), ()).is_some() {
                debug!(delivery_id, "GitHub webhook duplicate delivery, skipping");
                return StatusCode::OK;
            }
        }
    }

    // 6. Parse body
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "GitHub webhook body parse failed");
            return StatusCode::BAD_REQUEST;
        }
    };

    // 7. Bot self-event filter
    let event: GitHubWebhookEvent = match serde_json::from_value(payload.clone()) {
        Ok(e) => e,
        Err(_) => GitHubWebhookEvent { action: None, sender: None, installation: None },
    };
    if is_bot_self_event(&event, state.github_app_id) {
        debug!(event_type, "GitHub webhook bot self-event filtered");
        return StatusCode::OK;
    }

    // 8. Route to agent
    let check_conclusion = /* extract from check_suite payload if applicable */;
    let Some(target_agent) = route_event(event_type, event.action.as_deref(), check_conclusion) else {
        debug!(event_type, action = ?event.action, "GitHub webhook event not routable");
        return StatusCode::OK;
    };

    // 9. Semaphore
    let permit = match state.webhook_semaphore.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE,
    };

    // 10. Async dispatch
    let text = format_event_text(event_type, &payload);
    let request_id = delivery_id.to_string();
    tokio::spawn(async move {
        let _permit = permit;
        forward_github_event(&state, target_agent, &text, &request_id).await;
    });

    StatusCode::OK
}
```

#### Phase 3: Config and AppState Changes

**File: `crates/mika-gateway/src/settings.rs`**

```rust
// Add to GatewaySettings:
/// Secret for validating inbound GitHub App webhooks (HMAC-SHA256).
/// Optional -- when absent, /webhook/github returns 404.
#[serde(default)]
pub github_webhook_secret: Option<SecretString>,

/// GitHub App ID for bot self-event filtering.
/// Optional -- when absent, bot filtering uses sender.login heuristic.
#[serde(default)]
pub github_app_id: Option<u64>,
```

**File: `crates/mika-gateway/src/routes.rs`** (AppState)

```rust
// Add to AppState:
pub github_webhook_secret: Option<SecretString>,
pub github_app_id: Option<u64>,
pub github_delivery_cache: Arc<std::sync::Mutex<lru::LruCache<String, ()>>>,
```

#### Phase 4: Dependencies

**File: `Cargo.toml` (workspace root)**
- Add `hmac = "0.12"` to `[workspace.dependencies]`
- Add `lru = "0.12"` to `[workspace.dependencies]`

**File: `crates/mika-gateway/Cargo.toml`**
- Add `hmac.workspace = true`
- Add `lru.workspace = true`

#### Phase 5: Tests

**File: `crates/mika-gateway/src/github.rs` (inline tests)**

1. `test_validate_signature_valid` -- correct HMAC passes
2. `test_validate_signature_invalid` -- wrong body fails
3. `test_validate_signature_wrong_prefix` -- missing `sha256=` prefix fails
4. `test_validate_signature_empty` -- empty signature fails
5. `test_route_event_issues_opened` -- routes to mika-dev
6. `test_route_event_issues_assigned` -- routes to mika-dev
7. `test_route_event_issue_comment_created` -- routes to mika-dev
8. `test_route_event_pr_opened` -- routes to mika-qa
9. `test_route_event_pr_synchronize` -- routes to mika-qa
10. `test_route_event_pr_review_submitted` -- routes to mika-dev
11. `test_route_event_check_suite_failure` -- routes to mika-dev
12. `test_route_event_check_suite_success` -- returns None
13. `test_route_event_unknown` -- returns None
14. `test_route_event_ping` -- returns None
15. `test_is_bot_self_event_matching_app_id` -- filters bot events
16. `test_is_bot_self_event_different_app_id` -- does not filter
17. `test_is_bot_self_event_human_sender` -- does not filter
18. `test_is_bot_self_event_no_installation` -- does not filter

**Integration tests** (using `tower::ServiceExt::oneshot`):

19. `test_webhook_ping_returns_200` -- full handler with valid signature
20. `test_webhook_invalid_signature_returns_401`
21. `test_webhook_missing_signature_returns_401`
22. `test_webhook_unconfigured_returns_404` -- no `github_webhook_secret`
23. `test_webhook_duplicate_delivery_returns_200` -- second call with same delivery ID
24. `test_webhook_bot_self_event_filtered` -- bot event returns 200 without forwarding

#### Phase 6: Tracing and OpenAPI

- Add `#[utoipa::path(...)]` annotation to the handler
- Update `openapi.rs` to include the new endpoint
- Structured tracing fields in handler: `delivery_id`, `event_type`, `action`, `target_agent`

## System-Wide Impact

### Interaction Graph

GitHub sends POST to gateway -> HMAC validation -> LRU dedup check -> bot filter -> route_event() -> spawn async task -> POST to `{container_url}/message` with `channel: "github"`, `agent: "<target>"` -> agent container's `handle_message()` -> agent loop processes as a new message with `channel: "github"`.

### Error Propagation

- HMAC failure: 401 returned to GitHub (GitHub will retry)
- Body parse failure: 400 returned (GitHub will not retry on 4xx)
- Semaphore exhaustion: 503 returned (GitHub will retry)
- Container forwarding failure: logged in async task, no retry from gateway (GitHub handles retries)
- LRU cache lock poisoning: delivery_id check skipped (fail-open for availability), event processed normally

### State Lifecycle Risks

- **LRU cache on restart:** Delivery IDs lost. Acceptable: GitHub retries with same delivery ID are rare, and agent processing should be idempotent.
- **No Postgres state:** This phase adds no new tables. No migration concerns.

### API Surface Parity

- The `/webhook/github` endpoint parallels `/webhook/telegram` but with HMAC auth instead of header secret
- Both forward to `POST {container_url}/message` -- the message payload contract is the same
- OpenAPI spec must be updated

### Integration Test Scenarios

1. Valid signature + routable event -> message forwarded to correct agent container
2. Valid signature + unroutable event -> 200 OK, nothing forwarded
3. Invalid signature -> 401, nothing forwarded
4. Duplicate X-GitHub-Delivery -> first processed, second deduped
5. Gateway restart between duplicate deliveries -> both processed (acceptable)

## Acceptance Criteria

- [x] `POST /webhook/github` validates `X-Hub-Signature-256` via HMAC-SHA256
- [x] Ping events return 200 OK
- [x] Events are routed to correct agent names per the routing map
- [x] Bot self-events (matching `MIKA_GITHUB_APP_ID`) are filtered
- [x] Duplicate deliveries (same `X-GitHub-Delivery`) are deduplicated via LRU
- [x] Unroutable events return 200 OK and are logged at debug level
- [x] `MIKA_GITHUB_WEBHOOK_SECRET` is optional -- absent returns 404
- [x] Messages forwarded to agent containers use `channel: "github"` and `chat_id: 0`
- [x] Semaphore provides backpressure (503 when at capacity)
- [x] All new code has unit tests for validation, routing, and filtering
- [x] Integration tests cover the handler end-to-end
- [x] OpenAPI spec updated
- [x] Tracing spans include delivery_id, event_type, action, target_agent
- [x] Secret is redacted in Debug impls

## Dependencies & Risks

**New crate dependencies:**
- `hmac = "0.12"` -- HMAC-SHA256 computation (RustCrypto ecosystem, well-maintained)
- `lru = "0.12"` -- LRU cache for delivery dedup (widely used, minimal deps)

**Risks:**
- **Agent-side `chat_id: 0` handling**: The agent container may not gracefully handle `chat_id: 0`. The `GatewayMessageSender` uses `chat_id` for outbound replies. Mitigation: the agent should not call `send_message` for GitHub events (skill/prompt concern, not gateway concern). Document this as a known limitation for Phase 2b.
- **No multi-tenant routing**: Single-tenant only. Multi-tenant requires a Postgres mapping table and is deferred.
- **LRU cache not shared across replicas**: If the gateway is horizontally scaled, each replica has its own cache. Mitigated by GitHub's consistent delivery (same URL each time).

## Sources & References

- Issue: #382
- Parent plan: senara-solutions/mika-platform#3
- Phase 1 (prerequisite): #381 -- GitHub App JWT signing module in `mika-common`
- Telegram webhook pattern: `crates/mika-gateway/src/routes.rs`
- A2A proxy pattern: `crates/mika-gateway/src/a2a_routes.rs`
- Gateway settings: `crates/mika-gateway/src/settings.rs`
- GitHub App module: `crates/mika-common/src/github_app.rs`
- GitHub webhook docs: https://docs.github.com/en/webhooks
- GitHub webhook signature validation: https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries
