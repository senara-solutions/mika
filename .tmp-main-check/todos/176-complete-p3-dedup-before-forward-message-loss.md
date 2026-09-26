---
status: complete
priority: p3
issue_id: "176"
tags: [code-review, reliability]
---

# Address Dedup-Before-Forward Message Loss Risk

## Problem Statement
The atomic dedup claims the `update_id` BEFORE attempting to forward to the container (routes.rs:194-209). If the forward fails (container unreachable, timeout), the `update_id` is already claimed in the DB. Even if Telegram retries (if non-200 is returned per TODO #169), the dedup will reject the retry as "already processed." The user sees a transient error but the message is permanently lost.

## Findings
- **Security sentinel**: LOW severity — data durability issue
- **Note**: The old code updated last_update_id AFTER forwarding, which was race-prone but didn't lose messages on failure. The new atomic approach trades message loss on forward failure for race-condition elimination.

## Proposed Solutions

### Option A: Reset dedup on forward failure (Recommended)
If forward fails, roll back the last_update_id to allow retry:
```rust
Err(e) => {
    // Reset dedup so Telegram retry can succeed
    let _ = sqlx::query(
        "UPDATE customers SET last_update_id = last_update_id - 1 WHERE id = $1 AND last_update_id = $2"
    )
    .bind(row.id)
    .bind(update_id)
    .execute(&state.pool)
    .await;
    warn!(error = %e, customer_id = %row.id, "container unreachable, dedup reset");
    reply_transient_error(&state.telegram, chat_id).await;
}
```
Only works if Telegram retries (depends on TODO #169 fixing the 200 OK return).
- **Effort**: Small (15 min)
- **Risk**: Low — atomic CAS prevents incorrect rollback

### Option B: Move dedup after forward (accept duplicate risk)
Forward first, dedup after. The container's agent_lock provides idempotency.
- **Effort**: Small (10 min)
- **Risk**: Low — duplicate forwards are handled by agent mutex

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs:192-242`
- **Dependencies**: TODO #169 (webhook semaphore return code) — dedup reset only helps if Telegram retries

## Acceptance Criteria
- [ ] Messages not permanently lost when container is temporarily unreachable
- [ ] Dedup still prevents duplicate processing under normal operation

## Work Log
- 2026-02-24: Created from code review of commit 9de9ba6

## Resources
- Commit: 9de9ba6
- Related: TODO #169 (webhook semaphore return code)
