---
title: "SSE handler on a shared connection pool needs a subscriber semaphore"
date: 2026-05-18
category: architecture-patterns
module: gateway
problem_type: best_practice
component: http-server
severity: high
applies_when:
  - Adding an SSE / long-poll endpoint that runs a per-subscriber polling loop against a shared Postgres pool
  - The new endpoint shares its `PgPool` with other gateway responsibilities (webhook delivery, DLQ worker, A2A proxy)
  - Auth gates by token, not by subscriber identity — i.e., any authenticated caller can open arbitrary streams
  - You are about to wire a streaming endpoint into the existing axum router and reuse `state.pool`
tags:
  - sse
  - streaming
  - axum
  - postgres-pool
  - semaphore
  - backpressure
  - dos-resilience
  - resource-exhaustion
  - mika-1189
---

# SSE on a shared connection pool needs a subscriber semaphore

## Context

mika#1189 added a real-time orchestrator inbox to `mika-gateway`: spawned Claude Code tenants POST handoff messages and the orchestrator subscribes via `GET /orchestrator/inbox/{id}/stream` (SSE, long-poll, replay-from-cursor). The gateway already serves three other responsibilities — Telegram webhook delivery, the DLQ background worker, the A2A proxy — all sharing a single `PgPool` capped at `max_connections=20` in `crates/mika-gateway/src/main.rs`.

The natural first cut for an SSE handler is:

```rust
pub async fn handle_stream(State(state): State<AppState>, ...) -> Response {
    let stream = inbox_stream(state.pool.clone(), orchestrator_id, cursor);
    Sse::new(stream).keep_alive(...).into_response()
}
```

No cap on subscribers, no semaphore. Each subscriber's `async_stream` runs `fetch_after_cursor` (one Postgres query) every `ORCHESTRATOR_INBOX_POLL_INTERVAL` (1.5 s) for the life of the connection. The review's adversarial pass (ADV-001 / SEC-001) showed this collapses under modest load: ~25 simultaneous SSE streams + brief row backlogs saturate the 20-connection pool. The DLQ worker and webhook handlers then start failing `acquire_timeout(1s)`, cascading into webhook-delivery failures across the entire gateway — not just inbox streams.

The auth boundary doesn't help here: any caller with `MIKA_INTERNAL_TOKEN` can open unbounded streams, and in the operator-developer-infrastructure model that token is shared across spawn tenants. A misbehaving spawn (or a forgotten reconnect loop) becomes a gateway-wide outage trigger.

## Guidance

When an SSE / long-poll endpoint shares its `PgPool` with other gateway responsibilities, **cap concurrent subscribers via a `tokio::sync::Semaphore` whose permit is held for the lifetime of the stream**. Return `503 Service Unavailable` with a short `Retry-After` when the cap is hit. Mirror the existing `webhook_semaphore` pattern in `routes.rs`.

The three pieces:

1. **Permit acquisition at handler entry, before the stream is built.**

   ```rust
   let permit = match state
       .inbox_subscriber_semaphore
       .clone()
       .try_acquire_owned()
   {
       Ok(p) => p,
       Err(_) => {
           return (
               StatusCode::SERVICE_UNAVAILABLE,
               [("retry-after", "5")],
               Json(serde_json::json!({ "error": "at subscriber capacity; retry later" })),
           ).into_response();
       }
   };
   ```

   `try_acquire_owned()` (not `acquire`) means we don't wait — we shed load immediately. SSE clients reconnect on their own; pinning a request thread waiting for a permit just adds latency without improving throughput.

2. **Permit lifetime tied to the stream, not the handler.**

   The handler returns the `Sse` response and exits. The stream then runs for as long as the client is connected. The permit must travel with the stream, not be dropped at the end of the handler:

   ```rust
   fn inbox_stream(
       pool: PgPool,
       orch_id: String,
       cursor: i64,
       _permit: tokio::sync::OwnedSemaphorePermit,
   ) -> impl Stream<Item = Result<Event, Infallible>> {
       async_stream::stream! {
           let _hold = _permit;  // move into closure; dropped on stream drop
           let mut cursor = cursor;
           loop { /* poll + yield */ }
       }
   }
   ```

   When the client disconnects and axum drops the response body, `async_stream` cancels the generator and `_hold` drops, releasing the permit. No explicit cleanup path needed — this is the cheapest mechanism for "release-on-disconnect."

3. **Default cap derived from pool size, not picked arbitrarily.**

   Pool size 20; DLQ worker reserves a small budget; webhook delivery spikes can hold 30 permits. A cap of **10 SSE subscribers** keeps steady-state DB pressure well under the pool budget even when every subscriber has a backlog: `10 subscribers × 1 query/1.5s = 6.7 qps`, leaving 13+ connections for everything else. Encode the cap as a `pub const ORCHESTRATOR_INBOX_DEFAULT_SUBSCRIBER_CAP: usize = 10` with a comment explaining the budget math, and expose a `default_inbox_subscriber_semaphore()` constructor so callers can override for tests.

## Why This Matters

The Postgres connection pool is the gateway's shared scarce resource. Any new endpoint that holds a connection for longer than a single request — SSE, long-poll, streaming downloads, large reads — competes with every other endpoint for that resource.

A single unrelated endpoint with no concurrency cap can take down the whole gateway: webhook delivery (Telegram inbound, GitHub events), the DLQ retry worker, the A2A proxy, and the new SSE endpoint all share `state.pool`. The cascade is silent — error logs show `acquire_timeout` on the *innocent* paths while the *misbehaving* path is functioning normally from the client's perspective. Diagnosing this from logs alone is hard because the immediate-cause path looks healthy.

The semaphore turns this cascade into a clean failure mode: misbehaving callers get `503 + Retry-After`, innocent paths keep working. The same pattern protects the webhook handler (`webhook_semaphore: Semaphore::new(30)` in `main.rs`) — extending it to every new resource-holding endpoint is cheap defense.

## When to Apply

Apply this pattern when **all** of:

- The endpoint holds a `PgPool` connection (or any other shared resource) for the lifetime of the request, not just for one query
- The pool / resource is shared with other endpoint families
- Auth gates by token, not by per-subscriber identity (so a single compromised or misbehaving client can scale the resource usage)

You can **skip** the semaphore when:

- The endpoint is single-shot: acquire pool → run query → release pool → return JSON. The pool's internal scheduling handles fairness.
- The endpoint is gated by a per-subscriber/per-customer identity that already caps concurrency upstream (e.g., a per-customer connection limit in the LB).
- Holding the resource is bounded by the request body size (which has its own `RequestBodyLimitLayer` cap).

## Examples

**Before (mika#1189 first cut, ADV-001 finding):**

```rust
// handle_stream — no cap
let stream = inbox_stream(state.pool.clone(), orch_id, cursor);
Sse::new(stream).keep_alive(...).into_response()

fn inbox_stream(pool: PgPool, orch_id: String, cursor: i64)
    -> impl Stream<Item = Result<Event, Infallible>>
{
    async_stream::stream! {
        loop {
            let rows = fetch_after_cursor(&pool, &orch_id, cursor).await?;
            for row in rows { yield Ok(Event::default().data(serde(row))); }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}
```

**After (mika#1189 second commit, `crates/mika-gateway/src/orchestrator_inbox.rs`):**

```rust
// Acquire permit at handler entry — fail fast with 503 when at cap
let permit = match state
    .inbox_subscriber_semaphore
    .clone()
    .try_acquire_owned()
{
    Ok(p) => p,
    Err(_) => return (
        StatusCode::SERVICE_UNAVAILABLE,
        [("retry-after", SUBSCRIBER_CAP_RETRY_AFTER_SECS.to_string())],
        Json(serde_json::json!({ "error": "at subscriber capacity; retry later" })),
    ).into_response(),
};

let stream = inbox_stream(state.pool.clone(), orch_id, cursor, permit);
Sse::new(stream).keep_alive(...).into_response()

// Permit travels with the stream — dropped on disconnect via async_stream cancellation
fn inbox_stream(
    pool: PgPool,
    orch_id: String,
    cursor: i64,
    _permit: tokio::sync::OwnedSemaphorePermit,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let _hold = _permit;
        let mut cursor = cursor;
        loop {
            match fetch_after_cursor(&pool, &orch_id, cursor).await {
                Ok(rows) => { for row in rows { yield ...; mark_delivered(&pool, row.id).await; } }
                Err(e) => error!(error = %e, "poll failed"),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}
```

**Wire-up in main.rs and AppState:**

```rust
// crates/mika-gateway/src/routes.rs — AppState
pub struct AppState {
    pub pool: PgPool,
    pub webhook_semaphore: Arc<tokio::sync::Semaphore>,           // existing (30)
    pub inbox_subscriber_semaphore: Arc<tokio::sync::Semaphore>,  // new (10)
    // ...
}

// crates/mika-gateway/src/main.rs
let state = AppState {
    pool,
    webhook_semaphore: Arc::new(Semaphore::new(30)),
    inbox_subscriber_semaphore: orchestrator_inbox::default_inbox_subscriber_semaphore(),
    // ...
};
```

**Test for the 503 path (no real Postgres required):**

```rust
#[tokio::test]
async fn stream_returns_503_when_subscriber_cap_full() {
    // 1-permit semaphore, hold the permit, then attempt to subscribe.
    let state = test_state_with_subscriber_cap(true, 1);
    let held = state.inbox_subscriber_semaphore.clone().try_acquire_owned().unwrap();
    let app = build_router(state);
    let resp = app.oneshot(Request::get("/orchestrator/inbox/orch-1/stream")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(resp.headers().get("retry-after").unwrap(), "5");
    drop(held);
}
```

## Related

- `crates/mika-gateway/src/routes.rs` — `webhook_semaphore` reference (Telegram/GitHub webhook backpressure)
- `crates/mika-gateway/src/dlq.rs` — DLQ worker respects the same `webhook_semaphore` (`try_acquire_owned()` skip-on-full pattern)
- `docs/solutions/architecture/investigation-panel-sse-agent-loop.md` — the only prior SSE endpoint, uses `try_lock()` returning 429 for single-stream protection (different shape, same principle: shed load instead of queueing)
- `docs/solutions/best-practices/supervise-tokio-spawn-with-shutdown-flag-2026-05-16.md` — companion pattern for the retention task that complements the streaming endpoint
- mika#1189 plan — `docs/plans/2026-05-17-003-feat-1189-mika-gateway-orchestrator-inbox-v2-plan.md`
- mika#1189 review run — `.context/compound-engineering/ce-review/20260518-110934-b1793531/{adversarial,security,reliability}.json` (full evidence for the pool-exhaustion cascade)
