---
title: "Phase 2: Axum HTTP Server Architecture for Per-Customer Containers"
date: 2026-02-24
category: architecture-decisions
tags: [axum, http-server, async-rust, tokio, kubernetes, agent-architecture, send-sync]
severity: high
components: [server, messaging, tools, scheduler, async_db, auth]
related_issues: ["PR #5", "PR #3"]
---

# Phase 2: Axum HTTP Server Architecture for Per-Customer Containers

## Problem Statement

Mika's agent core (Phase 1) ran as a CLI binary with synchronous SQLite access. Phase 2 required transforming it into an HTTP server for Kubernetes deployment where each customer gets their own container. The core challenge: building an async Axum HTTP layer on top of a sync database agent without rewriting the entire stack.

Key constraints:
- `rusqlite::Connection` is `!Send` — cannot cross thread boundaries
- Tool trait futures were `?Send` — incompatible with `tokio::spawn`
- Agent loop must be serialized (one turn at a time per customer)
- Network failures must not lose outbound messages
- K8s heartbeat CronJobs must be cheap to reject

## Solution

### Challenge 1: Making Tool Futures Send-Compatible for tokio::spawn

**Problem:** The Tool trait used `#[async_trait(?Send)]`, making futures non-Send. Axum handlers need `tokio::spawn` for the agent loop, which requires Send futures.

**Root cause:** `ToolContext.db` held `&Database` wrapping a non-Send `rusqlite::Connection`.

**Solution:** Changed Tool trait to `#[async_trait]` (Send). Swapped `&Database` for `&AsyncDatabase` (Send+Sync wrapper using dedicated OS thread + mpsc channel). Updated all 8 tool implementations and the MessageSender trait.

```rust
// crates/mika-agent/src/tools/mod.rs
#[async_trait]
pub trait Tool: Send + Sync {
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput>;
}

pub struct ToolContext<'a> {
    pub db: &'a AsyncDatabase,  // Send+Sync wrapper, not &Database
    // ...
}
```

**Key insight:** The `?Send` was originally needed because `rusqlite::Connection` is `!Send`. The `AsyncDatabase` wrapper solved this by keeping the Connection on one dedicated OS thread, communicating via `Arc<Mutex<Database>>` + `tokio::task::spawn_blocking`.

### Challenge 2: Agent Serialization Without Blocking

**Problem:** Only one agent loop should run at a time (single-user container), but the HTTP server handles concurrent requests.

**Solution:** `tokio::sync::Mutex<()>` in AppState with non-blocking `try_lock_owned()`. Returns 429 "agent busy" immediately if lock unavailable — the gateway can retry. Heartbeat handler returns 204 (skippable). Lock held for the duration of the spawned agent loop.

```rust
// crates/mika-agent/src/server/handlers.rs
let lock = match state.agent_lock.clone().try_lock_owned() {
    Ok(guard) => guard,
    Err(_) => return (StatusCode::TOO_MANY_REQUESTS,
        Json(json!({"error": "agent busy"}))).into_response(),
};

tokio::spawn(async move {
    let _lock = lock; // Hold lock for duration of agent loop
    // ... run agent ...
});
```

**Design decision:** Non-blocking try_lock with 429 backpressure instead of queuing. In a single-user container, the gateway is the only caller and can implement retry logic.

### Challenge 3: Outbound Message Delivery with Resilience

**Problem:** Agent responses must reach the user via a gateway. Network failures shouldn't lose messages or confuse the agent.

**Solution:** `GatewayMessageSender` POSTs to gateway `/send` with retry: first attempt -> 2s backoff -> retry -> on failure, save to `failed_sends` DB table and return `Ok(())`. Returning Ok prevents the agent from entering an error state; the message is queued for later. Failed sends flushed (up to 5) at the start of each subsequent message handler.

```rust
// crates/mika-agent/src/messaging.rs
Err(e) => {
    warn!(error = %e, "retry failed, saving to failed_sends");
    self.db.save_failed_send(text, None).await?;
    Ok(()) // Return Ok — message queued, don't confuse Claude
}
```

**Key insight:** The agent thinks the send succeeded, so it won't retry internally. The `failed_sends` table acts as a durable outbox, flushed opportunistically on the next user message.

### Challenge 4: Heartbeat Pre-Filtering Without Wasting Resources

**Problem:** K8s CronJob fires heartbeats frequently. Most should be skipped without acquiring the Mutex or calling Claude.

**Solution:** 4-check pre-filter in `heartbeat_should_run()` using only cheap DB queries:
1. Active hours (8-21 in customer's local timezone via chrono-tz)
2. Max 1 heartbeat per hour
3. Max 3 heartbeats per day
4. Skip if user messaged within 2 hours

All checks run BEFORE `try_lock`. Returns `204 No Content` immediately if any check fails.

### Challenge 5: Bearer Token Auth with Timing-Safety

**Problem:** Token comparison must be constant-time to prevent timing attacks.

**Solution:** `subtle::ConstantTimeEq` for byte-level constant-time comparison in Axum middleware.

```rust
// crates/mika-agent/src/server/auth.rs
match token {
    Some(t) if bool::from(t.as_bytes().ct_eq(state.internal_token.as_bytes())) => {
        next.run(req).await.into_response()
    }
    _ => StatusCode::UNAUTHORIZED.into_response(),
}
```

### Challenge 6: ReminderScheduler Ownership for Arc Wrapping

**Problem:** ReminderScheduler had lifetime parameters (`&'a AsyncDatabase`) preventing `Arc<ReminderScheduler>` (Arc requires `'static`).

**Solution:** Changed to owned types: `AsyncDatabase` (Clone), `ClaudeClient` (added Clone derive), `Arc<ToolRegistry>`. Removed all lifetime parameters.

```rust
// crates/mika-agent/src/scheduler.rs
pub struct ReminderScheduler {
    pub db: AsyncDatabase,           // Owned, Clone
    pub claude: ClaudeClient,        // Owned, Clone (derive added)
    pub tools: Arc<ToolRegistry>,    // Arc (already 'static)
    pub home_dir: PathBuf,           // Owned
    pub message_sender: Option<Arc<dyn MessageSender>>,
}
```

## Architecture Overview

```
Gateway (shared) ──POST /message──> Container (per-customer)
                                      │
                                      ├── Axum Router
                                      │   ├── GET /health (no auth, K8s probes)
                                      │   ├── POST /message (Bearer auth, 202 async)
                                      │   └── POST /heartbeat (Bearer auth, CronJob)
                                      │
                                      ├── AppState
                                      │   ├── AsyncDatabase (Send+Sync wrapper)
                                      │   ├── ClaudeClient
                                      │   ├── Arc<ToolRegistry>
                                      │   ├── Arc<ReminderScheduler>
                                      │   └── Arc<Mutex<()>> (agent lock)
                                      │
                                      └── Agent Loop (spawned task, holds lock)
                                          ├── Prompt assembly
                                          ├── Claude API call
                                          ├── Tool execution (8 tools)
                                          └── GatewayMessageSender ──POST /send──> Gateway
```

## Lessons Learned

### 1. ?Send vs Send Is a Foundational Decision

The `?Send` trait bound on async traits propagates through the entire call chain. Changing it later requires touching every file that uses the trait. Make the Send/Sync decision early based on your deployment model (single-threaded CLI vs multi-threaded server).

### 2. Axum Middleware Layering Order Matters

`route_layer` applies to routes defined ABOVE it in the chain, not below. Health endpoint must be added AFTER the auth route_layer to bypass authentication. Getting this wrong silently applies auth to K8s probes.

```rust
Router::new()
    .route("/message", post(handle_message))  // auth applied
    .route("/heartbeat", post(handle_heartbeat))  // auth applied
    .route_layer(middleware::from_fn_with_state(state, require_internal_token))
    .route("/health", get(handle_health))  // NO auth (below route_layer)
    .with_state(state)
```

### 3. Audit Inline Side-Effects When Adding Entry Points

The CLI ran compaction inline after each agent turn. The HTTP server also spawned compaction in the handler. Result: double compaction per message in server mode. Every inline side-effect in the original entry point must be audited when adding a new one.

### 4. Stale Future-Work Comments Become Lies

`message_sender: None // Wired in PR 3` shipped in PR 3 — still None. Use `// TODO:` with a specific condition, not PR references that become stale.

### 5. Atomic Ordering Requires Pairing

`Release` store + `Relaxed` load is incorrect. On ARM, the Relaxed load may never see the Release store. Always pair Release with Acquire.

### 6. Channel Filters Should Default to Open

Hardcoding `["cli", "telegram"]` silently excludes future channels. In a single-user container, all channels belong to the same person. Default to `None` (all channels).

## Prevention Strategies

| Pattern | Rule | Verification |
|---------|------|--------------|
| Sync-to-async wrapping | Keep `!Send` types on dedicated thread, expose Send+Sync wrapper | Test that wrapper compiles in `tokio::spawn` |
| Middleware ordering | Health routes AFTER auth `route_layer` | Test unauthenticated health access |
| Side-effect deduplication | Audit all CLI side-effects before adding HTTP entry point | Grep for duplicate `maybe_compact` / `recover` calls |
| Comment hygiene | No PR references in comments — use `// TODO(scope):` | `grep -r "PR #" crates/` should be empty |
| Atomic ordering | Release store pairs with Acquire load | Run under Miri on nightly |
| Channel filters | Default to `None` (allow all) in single-user containers | Test with no allowlist set |

## Files Changed

| File | Change |
|------|--------|
| `crates/mika-agent/src/server/mod.rs` | Server entry point, router, graceful shutdown |
| `crates/mika-agent/src/server/handlers.rs` | Health, message, heartbeat handlers |
| `crates/mika-agent/src/server/auth.rs` | Bearer token middleware (subtle) |
| `crates/mika-agent/src/server/state.rs` | AppState with Arc-wrapped deps |
| `crates/mika-agent/src/server/types.rs` | Request/response types |
| `crates/mika-agent/src/messaging.rs` | GatewayMessageSender, MessageSender trait |
| `crates/mika-agent/src/tools/mod.rs` | Tool trait ?Send -> Send, ToolContext |
| `crates/mika-agent/src/agent.rs` | AgentParams with AsyncDatabase |
| `crates/mika-agent/src/scheduler.rs` | Owned types, no lifetime params |
| `crates/mika-agent/src/async_db.rs` | Panic resilience (catch_unwind) |
| `crates/mika-common/src/config.rs` | Server-mode settings |

## Related Documents

- [Implementation plan](../../plans/2026-02-24-feat-phase2-container-http-server-plan.md)
- [AsyncDatabase wrapper pattern](../architecture/async-database-wrapper-pattern.md)
- [Platform systems brainstorm](../../brainstorms/2026-02-24-platform-systems-brainstorm.md)
- PR #5: Phase 2 Container HTTP Server
- PR #3: Platform Systems Phase 1 (foundation)

## Code Review Findings

A 6-agent parallel review (security, performance, architecture, simplicity, agent-native, learnings) produced 18 findings:

- **2 P1 (Critical):** Channel filter excludes WhatsApp (#118), ReminderScheduler has no MessageSender (#119)
- **9 P2 (Important):** Double compaction (#120), reqwest::Client per send (#121), token not redacted (#122), atomic ordering (#123), flush blocks processing (#124), dead code (#125-126), timezone ignored (#127), no JSON on 401 (#128)
- **7 P3 (Nice-to-have):** Import ordering, unused fields, test duplication, observability gaps (#129-135)

All findings tracked in `todos/118-135-pending-*.md`.
