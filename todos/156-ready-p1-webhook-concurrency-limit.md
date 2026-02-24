---
status: complete
priority: p1
issue_id: "156"
tags: [code-review, security, performance]
---

# Add Concurrency Limit on Webhook Spawned Tasks

## Problem Statement
Every incoming webhook spawns an unbounded `tokio::spawn` task (routes.rs:97). Under burst traffic, this can exhaust the Postgres connection pool (max 10), consume unbounded memory, and cascade failures as error paths generate more Telegram API calls. Flagged by security and performance agents as the primary scalability bottleneck.

## Findings
- **Security sentinel**: DoS via webhook flooding; each task queries Postgres and may call Telegram API
- **Performance oracle**: At 500 msgs/sec, 490 tasks block on pool acquire, each consuming memory; error paths cascade with Telegram replies

## Proposed Solutions

### Option A: tokio::sync::Semaphore (Recommended)
```rust
// In AppState:
pub webhook_semaphore: Arc<tokio::sync::Semaphore>,
// In handle_webhook:
let permit = match state.webhook_semaphore.try_acquire() {
    Ok(p) => p,
    Err(_) => { warn!("at capacity, shedding load"); return StatusCode::OK; }
};
tokio::spawn(async move { let _permit = permit; /* process */ });
```
Set limit to 20-30 (2-3x pool size). Shed excess load silently (Telegram retries).
- Pros: Simple, bounded, protects pool
- Cons: Dropped messages under extreme load (acceptable — Telegram retries)
- Effort: Small (15 min)
- Risk: Low

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs` (lines 96-127, AppState struct)

## Acceptance Criteria
- [ ] Semaphore added to AppState with configurable limit
- [ ] try_acquire used (non-blocking)
- [ ] Excess requests return 200 to Telegram (shed silently)
- [ ] All existing tests pass

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
