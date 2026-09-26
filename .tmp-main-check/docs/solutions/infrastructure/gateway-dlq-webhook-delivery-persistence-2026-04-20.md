---
title: Gateway dead-letter queue for exhausted webhook retries
date: 2026-04-20
category: infrastructure
module: mika-gateway
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding retry-with-backoff to any delivery pipeline where events must not be lost
  - Gateway or relay service handles webhook forwarding to downstream containers
  - Downstream services may be down for extended periods (>5 minutes)
tags: [dlq, dead-letter-queue, webhook, retry, gateway, postgres, background-worker]
---

# Gateway Dead-Letter Queue for Exhausted Webhook Retries

## Context

The gateway routes GitHub webhook events to per-customer agent containers. Issue #589 added retry-with-backoff (`[2s, 5s, 15s, 60s, 300s]` with ±25% jitter) for transient failures. However, pathological cases — agent down for 10+ minutes, gateway restart while retries are in-flight, sustained backpressure exhausting the retry budget — result in permanent event loss with only an ERROR log.

This pattern adds persistence at the exhaustion boundary, turning "event dropped" into "event queued for later delivery."

## Guidance

**Architecture:** Single Postgres table (`webhook_deliveries`) stores the formatted delivery payload, metadata, and status. A background tokio task wakes every 30s to retry pending entries with exponential backoff (`30s * 2^attempts`, capped at 1h). After 10 worker attempts, status transitions to `dead`. CLI and HTTP endpoints allow operator inspection and manual replay.

**Key design decisions:**

1. **Store formatted text, not raw webhook body** — The DLQ stores the already-formatted message text (`~1KB`) rather than the raw GitHub body (`~256KB`). This keeps storage compact and avoids re-parsing on replay.

2. **Re-resolve routes on replay** — Container URLs may change between failure and replay (containers moved, recreated). The worker and replay endpoints call `resolve_github_container_url()` fresh rather than caching the original URL.

3. **CLI talks to gateway via HTTP, not direct Postgres** — The CLI runs on user machines; the gateway runs in K8s. New REST endpoints (`GET /webhook/dlq`, `POST .../replay`, `POST .../replay-all`) expose DLQ operations behind the same internal token auth as `/send`.

4. **Fire-and-forget DLQ insert** — The insert into `webhook_deliveries` at the retry exhaustion point logs errors but never propagates them. If the DB write fails, the original ERROR log still fires.

5. **Semaphore respect** — The background worker and replay endpoints use `try_acquire_owned()` on the shared 30-permit webhook semaphore, skipping entries when at capacity rather than blocking.

**Integration points:**
- Write path: two terminal failure points in `deliver_with_retry_inner()` (retry budget exhausted, semaphore capacity)
- Worker: spawned in `main.rs` before `axum::serve()`
- HTTP endpoints: registered in `build_router()` with `require_bearer_token` middleware
- CLI: `mika webhook list-dead|replay|replay-all` via `MIKA_GATEWAY_URL`

## Why This Matters

Without persistence at the retry exhaustion boundary, events are permanently lost during extended agent downtime or gateway restarts. The DLQ ensures zero-event-loss semantics for the gateway's inbound delivery path with minimal complexity (one table, one background task, one CLI subcommand group).

## When to Apply

- Any delivery pipeline where the retry budget is bounded and events must survive exhaustion
- When the downstream service has extended downtime windows (maintenance, scaling events)
- When the gateway process itself may restart with in-flight retries

## Examples

**DLQ insert at retry exhaustion:**
```rust
// In deliver_with_retry_inner(), after all retries exhausted:
crate::dlq::insert_delivery(
    &state.pool,
    crate::dlq::NewDelivery {
        delivery_id: request_id,
        event_type,
        target_agent,
        repo_full_name,
        payload: text,
        request_id,
        attempts: attempts_made as i32,
        last_error: &last_reason,
    },
).await;
```

**Background worker backoff query:**
```sql
SELECT * FROM webhook_deliveries
WHERE status = 'pending'
  AND (last_attempt_at IS NULL
       OR last_attempt_at < now() - (LEAST(30 * power(2, attempts), 3600) || ' seconds')::interval)
ORDER BY created_at ASC
LIMIT 50
```

## Related Issues

- #590 — This feature (dead-letter queue + replay CLI)
- #589 — Prerequisite (retry with backoff)
- #583 / PR #586 — Engine-side dispatch guards (symmetric concept on the agent side)
