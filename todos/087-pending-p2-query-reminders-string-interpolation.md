---
status: pending
priority: p2
issue_id: "087"
tags: [code-review, security, architecture]
dependencies: []
---

# Replace query_reminders string interpolation with type-safe enum

## Problem Statement
`query_reminders()` in db.rs constructs SQL by concatenating an `extra_where: &str` parameter via `format!()`. While all 3 callers pass hardcoded string literals, the method signature accepts arbitrary strings, creating a fragile pattern that could become a SQL injection vector if a future caller passes user input.

## Findings
- File: `crates/mika-agent/src/db.rs:674-695`
- Method is private (`fn` not `pub`), limiting blast radius
- All 3 callers pass static literals: `""`, `" AND fire_at > datetime('now')"`, `" AND fire_at <= datetime('now')"`
- Pattern conflicts with the project's universal parameterized-query convention (every other query uses `?N` placeholders)
- Also prevents SQLite statement caching since each call produces a different SQL string
- Flagged by: Security Sentinel, Architecture Strategist, Pattern Recognition, Performance Oracle

## Proposed Solutions

### Option 1: ReminderFilter enum (Recommended)
```rust
enum ReminderFilter { All, Future, PastDue }
fn query_reminders(&self, filter: ReminderFilter) -> Result<Vec<Reminder>> {
    let sql = match filter {
        ReminderFilter::All => "SELECT ... WHERE status = 'pending' ORDER BY fire_at ASC",
        ReminderFilter::Future => "SELECT ... WHERE status = 'pending' AND fire_at > datetime('now') ORDER BY fire_at ASC",
        ReminderFilter::PastDue => "SELECT ... WHERE status = 'pending' AND fire_at <= datetime('now') ORDER BY fire_at ASC",
    };
    let mut stmt = self.conn.prepare_cached(sql)?;
    // ...
}
```
**Pros:** Type-safe, enables statement caching, impossible to inject
**Cons:** Slightly more code
**Effort:** Small
**Risk:** Low

### Option 2: Safety comment
Add `/// SAFETY: extra_where must be a compile-time constant` doc comment.
**Pros:** Zero code change
**Cons:** Relies on discipline, doesn't fix statement caching
**Effort:** Trivial
**Risk:** Medium (pattern remains fragile)

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs`

## Acceptance Criteria
- [ ] `query_reminders` no longer accepts arbitrary `&str`
- [ ] All 3 callers updated to use enum variant
- [ ] `prepare_cached` used for statement reuse
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified fragile SQL construction pattern in query_reminders helper
