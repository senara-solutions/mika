---
status: complete
priority: p2
issue_id: "205"
tags: [code-review, quality, architecture]
dependencies: []
---

# Init Code Duplication and Missing Cleanup Guard

## Problem Statement

Two issues: (1) `init()` and `init_db_only()` share 80% of their logic but are fully duplicated. (2) All command handlers call `ctx.async_db.shutdown()` manually — if any returns early via `?`, the DB thread leaks.

## Findings

- **Source:** architecture-strategist (Finding 3), code-simplicity-reviewer (Finding 7c), pattern-recognition-specialist (Finding 6b)
- **Location:** `crates/mika-cli/src/init.rs:26-68`, all command files
- **Evidence:** Lines 26-48 and 52-68 are nearly identical. Five commands each have explicit `shutdown()` calls that can be skipped on early error return.

## Proposed Solutions

### Option 1: Extract shared init_base() + implement Drop on contexts
- **Pros**: Eliminates duplication; Drop ensures cleanup even on error paths
- **Cons**: Drop cannot be async, but `shutdown()` is synchronous so it works
- **Effort**: Small
- **Risk**: Low

```rust
fn init_base() -> Result<(Settings, AsyncDatabase, PathBuf)> { ... }
pub fn init() -> Result<AppContext> { let (s, db, h) = init_base()?; /* + claude */ }
pub fn init_db_only() -> Result<DbContext> { let (s, db, h) = init_base()?; ... }

impl Drop for DbContext {
    fn drop(&mut self) { self.async_db.shutdown(); }
}
```

## Recommended Action

Option 1. Also fixes the `&PathBuf` clippy lint in `ensure_initialized` (should be `&Path`).

## Technical Details

- **Affected files:** `crates/mika-cli/src/init.rs`, all command files (remove manual shutdown calls)

## Acceptance Criteria

- [ ] `init()` calls `init_db_only()` internally — no duplicated logic
- [ ] DB shutdown happens automatically via Drop, even on error paths
- [ ] `ensure_initialized` takes `&Path` not `&PathBuf`
- [ ] Manual `shutdown()` calls removed from command files

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | |

## Resources

- Commit: 399ebf0
