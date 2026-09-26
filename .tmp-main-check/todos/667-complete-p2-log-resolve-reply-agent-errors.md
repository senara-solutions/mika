---
status: pending
priority: p2
issue_id: 667
tags: [code-review, observability, gateway]
dependencies: []
---

# Add warn! Logging on resolve_reply_agent DB Errors

## Problem Statement

`resolve_reply_agent` swallows DB errors via `.ok().flatten()`. DB errors during reply lookup could indicate connection issues or schema problems, and should be logged for operational visibility — matching the pattern used by `resolve_customer`.

## Findings

- `crates/mika-gateway/src/routes.rs:719-728` — `.ok()` silently discards errors
- `crates/mika-gateway/src/routes.rs:220-245` — `resolve_customer` logs `warn!` on DB errors

Identified by: pattern-recognition-specialist, security-sentinel

## Proposed Solutions

Replace `.ok().flatten()` with explicit match and `warn!`:

```rust
match sqlx::query_scalar::<_, String>(...).fetch_optional(&state.pool).await {
    Ok(opt) => opt,
    Err(e) => {
        warn!(error = %e, chat_id, telegram_message_id = msg_id, "reply agent lookup failed");
        None
    }
}
```

- **Effort**: Small
- **Risk**: None
