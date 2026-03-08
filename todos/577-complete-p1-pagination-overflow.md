---
status: pending
priority: p1
issue_id: "577"
tags: [code-review, security]
dependencies: []
---

# Pagination Arithmetic Overflow in resolve_pagination

## Problem Statement
In `resolve_pagination()`, the expression `(page - 1) * per_page` overflows for large page values. With `page = u32::MAX` and `per_page = 200`, this panics in debug builds (DoS) or wraps around in release builds (returns wrong data).

## Findings
- **Source:** Security Sentinel + Performance Oracle
- **Location:** `crates/mika-agent/src/server/dashboard.rs` lines 33-38

## Proposed Solutions

### Option A: Checked arithmetic with page cap
```rust
fn resolve_pagination(page: Option<u32>, per_page: Option<u32>) -> (u32, u32, u32) {
    let page = page.unwrap_or(1).clamp(1, 100_000);
    let per_page = per_page.unwrap_or(DEFAULT_PER_PAGE).clamp(1, MAX_PER_PAGE);
    let offset = (page - 1).saturating_mul(per_page);
    (page, per_page, offset)
}
```
- **Effort:** Small (one-line fix)
- **Risk:** None

## Acceptance Criteria
- [ ] `page=u32::MAX` does not panic or return wrong data
- [ ] Unit test covers overflow case

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Security Sentinel found overflow |

## Resources
- PR #89
