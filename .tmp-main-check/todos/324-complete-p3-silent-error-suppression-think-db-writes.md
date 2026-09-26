---
status: complete
priority: p3
issue_id: "324"
tags: [code-review, quality, observability]
dependencies: []
---

# Silent Error Suppression on Thinking Level DB Writes

## Problem Statement

Both `set_customer_config` calls in `handle_think` use `let _ = ...` to silently
discard DB write errors:

```rust
let _ = app.db.set_customer_config("thinking_level", "off").await;   // line 636
let _ = app.db.set_customer_config("thinking_level", l).await;       // line 651
```

The in-memory state is updated regardless, so the current session works fine. But if
the DB write fails (disk full, DB locked), the user sees "Thinking level: high" yet
the preference won't persist on restart. There's no diagnostic trail.

## Findings

- **Architecture reviewer:** "A `tracing::warn!` on write failure would match the pattern used in other non-critical DB operations like reminder recovery."
- **Security reviewer:** "Consider logging the error at `tracing::warn!` level so it is observable in diagnostics."

## Proposed Solutions

### Option A: Log on failure (Recommended)

```rust
if let Err(e) = app.db.set_customer_config("thinking_level", l).await {
    tracing::warn!(error = %e, "failed to persist thinking level");
}
```

- **Pros:** Observable in logs, matches project patterns
- **Cons:** Two extra lines per call site
- **Effort:** Trivial
- **Risk:** None

### Option B: Leave as-is

- **Pros:** Simpler code
- **Cons:** Silent failures are invisible
- **Effort:** None
- **Risk:** Low

## Technical Details

- **File:** `crates/mika-cli/src/tui/commands/handlers.rs` lines 636, 651

## Acceptance Criteria

- [ ] DB write failures are logged at warn level
- [ ] In-memory state still updates regardless of DB write success

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from code review | Prefer tracing::warn over let _ for observable failures |
