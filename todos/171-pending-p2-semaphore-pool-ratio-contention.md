---
status: pending
priority: p2
issue_id: "171"
tags: [code-review, performance, reliability]
---

# Align Webhook Semaphore with Postgres Pool Size

## Problem Statement
The webhook semaphore allows 30 concurrent tasks, but the Postgres pool has only 10 connections (1s acquire timeout). Each text message task executes 2 DB queries (SELECT + UPDATE). Under burst load of 30 concurrent webhooks, 20 tasks wait for pool connections. If wait exceeds 1s `acquire_timeout`, the query fails, hitting the `Err(e)` branch in dedup, silently dropping the message.

## Findings
- **Performance oracle**: Critical tuning issue — 3:1 ratio can cause pool exhaustion and silent message drops
- **Security sentinel**: Pool contention can mask dedup failures, leading to data loss

## Proposed Solutions

### Option A: Raise pool to 20 connections (Recommended)
```rust
.max_connections(20)
```
With 30 concurrent tasks and 2 queries each, 20 connections provides headroom. Postgres can easily handle 20 connections.
- **Effort**: Trivial (2 min)
- **Risk**: None — 20 connections is modest for Postgres

### Option B: Lower semaphore to 15
```rust
webhook_semaphore: Arc::new(tokio::sync::Semaphore::new(15))
```
Matches pool capacity better: 15 tasks * 2 queries <= 30 connection acquisitions, but only ~15 concurrent at any given time.
- **Effort**: Trivial (2 min)
- **Risk**: Lower throughput ceiling

### Option C: Make both configurable
Add `webhook_max_concurrency` and `database_max_connections` to GatewaySettings.
- **Effort**: Small (15 min)
- **Risk**: None

## Technical Details
- **Affected files**: `crates/mika-gateway/src/main.rs:30` (pool) and `main.rs:69` (semaphore)

## Acceptance Criteria
- [ ] Semaphore permits and pool connections are balanced (no pool exhaustion under full semaphore load)
- [ ] No silent message drops due to pool acquire timeout

## Work Log
- 2026-02-24: Created from code review of commit 9de9ba6

## Resources
- Commit: 9de9ba6
