---
status: complete
priority: p3
issue_id: "323"
tags: [code-review, quality, duplication]
dependencies: []
---

# Duplicated Thinking Level Load Logic in chat.rs

## Problem Statement

The 6-line thinking level load-from-DB block is copy-pasted in two places in
`crates/mika-cli/src/commands/chat.rs`:

1. Lines 226-234 (startup)
2. Lines 316-322 (agent switch)

Both do the same thing: reset thinking_level to None, read from DB, resolve, set.

## Findings

- **Simplicity reviewer:** "This should be a small helper method on `App`. Both call sites become `app.load_thinking_level().await;` — one line instead of six. Eliminates ~10 lines of duplication."

## Proposed Solutions

### Option A: Extract method on App (Recommended)

Add to `crates/mika-cli/src/tui/app.rs`:

```rust
pub async fn load_thinking_level(&mut self) {
    self.thinking_level = None;
    if let Ok(Some(level_str)) = self.db.get_customer_config("thinking_level").await
        && let Some(resolved) = crate::tui::commands::resolve_thinking_level(&level_str)
    {
        self.thinking_level = Some(resolved);
    }
}
```

- **Pros:** Single source of truth, ~10 LOC reduction
- **Cons:** None
- **Effort:** Small
- **Risk:** None

### Option B: Leave as-is

- **Pros:** No additional change
- **Cons:** Two identical blocks to maintain
- **Effort:** None
- **Risk:** Low (blocks could diverge over time)

## Technical Details

- **File:** `crates/mika-cli/src/commands/chat.rs` lines 226-234 and 316-322
- **Note:** Startup path uses `worker._ctx.async_db` while agent switch uses `app.db`. Verify they are the same reference at that point (they should be — `app.db` is cloned from `worker._ctx.async_db` at App::new).

## Acceptance Criteria

- [ ] Single method handles thinking level loading
- [ ] Both call sites use the same method
- [ ] Startup and agent switch behavior unchanged

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-28 | Created from code review | Extract helper when same logic appears in 2+ places |
