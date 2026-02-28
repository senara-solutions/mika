# ADR-001: Axum HTTP Server Architecture for Per-Customer Containers

**Date:** 2026-02-24
**Status:** Accepted
**Components:** server, messaging, tools, scheduler, async_db, auth

## Context

Mika's agent core ran as a CLI binary with synchronous SQLite access. Hosted mode
required transforming it into an HTTP server for Kubernetes deployment where each
customer gets their own container. The core challenge: building an async Axum HTTP
layer on top of a sync database without rewriting the entire stack.

Key constraints:
- `rusqlite::Connection` is `!Send` — cannot cross thread boundaries
- Tool trait futures were `?Send` — incompatible with `tokio::spawn`
- Agent loop must be serialized (one turn at a time per customer)
- Network failures must not lose outbound messages
- Heartbeat CronJobs must be cheap to reject

## Decision

### Making Tool Futures Send-Compatible

Changed Tool trait from `#[async_trait(?Send)]` to `#[async_trait]` (Send). Swapped
`&Database` for `&AsyncDatabase` — a Send+Sync wrapper using a dedicated OS thread
and mpsc channel. The Connection stays on one thread; callers communicate via closures
sent over the channel.

### Agent Serialization Without Blocking

`tokio::sync::Mutex<()>` with non-blocking `try_lock_owned()`. Returns 429 "agent
busy" immediately if the lock is unavailable. The gateway retries. Heartbeat handler
returns 204 (skippable).

### Outbound Message Resilience (Durable Outbox)

`GatewayMessageSender` retries once with 2s backoff. On second failure, saves to
`failed_sends` table and returns `Ok(())` — the agent thinks the send succeeded,
preventing error loops. Failed sends are flushed (up to 5) at the start of each
subsequent message handler.

### Heartbeat Pre-Filtering

Four checks run before acquiring the Mutex (cheap, no Claude API call):
1. Active hours (8-21 in customer's local timezone)
2. Max 1 heartbeat per hour
3. Max 3 heartbeats per day
4. Skip if user messaged within 2 hours

### Bearer Token Auth

`subtle::ConstantTimeEq` for timing-safe token comparison in Axum middleware.

### ReminderScheduler Ownership

Changed from lifetime parameters to owned types (`AsyncDatabase` Clone,
`ClaudeClient` Clone, `Arc<ToolRegistry>`) to enable `Arc<ReminderScheduler>`.

## Consequences

- All tool implementations require Send+Sync bounds
- `?Send` vs Send is a foundational decision — changing it later touches every file
- Axum middleware ordering matters: health routes must be added after `route_layer`
  to bypass authentication
- Every CLI-path side-effect must be audited when adding HTTP entry points (e.g.,
  double compaction was found and fixed)
- `Release` store must pair with `Acquire` load (not `Relaxed`) for correctness on ARM
