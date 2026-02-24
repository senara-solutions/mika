---
status: ready
priority: p1
issue_id: "155"
tags: [code-review, security, data-integrity, race-condition]
---

# Make Dedup Atomic via Conditional UPDATE

## Problem Statement
The dedup logic in `handle_text_message` (routes.rs:167-217) uses a non-atomic read-then-check pattern. Two concurrent tasks processing the same `update_id` can both pass the check before either writes, causing duplicate message delivery to the container. Flagged by security, data integrity, and performance agents.

## Findings
- **Security sentinel**: TOCTOU race between SELECT and UPDATE; recommends atomic `UPDATE...WHERE last_update_id < $1`
- **Data integrity guardian**: Read-then-check without transaction; critical for multi-replica deployments
- **Performance oracle**: Reinforced by unbounded spawns — concurrent tasks amplify the race window

## Proposed Solutions

### Option A: Atomic conditional UPDATE before forwarding (Recommended)
Move dedup claim before container forward:
```rust
let claimed = sqlx::query("UPDATE customers SET last_update_id = $1 WHERE id = $2 AND last_update_id < $1 RETURNING id")
    .bind(update_id).bind(row.id).fetch_optional(&state.pool).await;
match claimed {
    Ok(Some(_)) => { /* forward to container */ }
    Ok(None) => return, // already processed
    Err(e) => { /* handle error */ }
}
```
- Pros: Atomic, correct for multi-replica, simple
- Cons: Marks as processed before confirmed delivery (changes semantics — failed forwards won't retry via dedup)
- Effort: Small (30 min)
- Risk: Low — container has its own idempotency via request_id

### Option B: Keep current pattern, add dedup at container level
- Pros: No gateway change
- Cons: Doesn't fix the race, relies on container-side dedup
- Effort: None at gateway
- Risk: Medium — duplicate messages still reach container

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs` (lines 142-217)

## Acceptance Criteria
- [ ] Dedup check and last_update_id update happen in a single SQL statement
- [ ] Concurrent requests for same update_id result in at-most-once forwarding
- [ ] All existing tests pass

## Work Log
- 2026-02-24: Created from PR #6 code review

## Resources
- PR: #6
