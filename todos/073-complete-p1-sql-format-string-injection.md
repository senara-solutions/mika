---
status: pending
priority: p1
issue_id: "073"
tags: [code-review, security]
dependencies: []
---

# Fix SQL format string in prune_old_heartbeat_sends

## Problem Statement
`prune_old_heartbeat_sends` in db.rs:801 uses `format!()` to interpolate `days: u32` directly into a SQL string instead of using parameterized queries. While `u32` prevents actual injection, this violates the project's universal parameterized-query convention and creates a dangerous precedent.

## Findings
- File: `crates/mika-agent/src/db.rs:800-804`
- Only query in the entire codebase using string interpolation for SQL
- Every other query (30+) uses `?1`, `?2` parameterized placeholders
- If `days` type is ever widened to `&str`, this becomes real SQL injection
- Flagged by: Security Sentinel (P1), Performance Oracle (P1), Pattern Recognition (P1), Architecture Strategist (P2)

## Proposed Solutions

### Option 1: Parameterize the modifier string (Recommended)
Compute the modifier in Rust and pass as parameter:
```rust
let modifier = format!("-{days} days");
self.conn.execute(
    "DELETE FROM heartbeat_sends WHERE sent_at < datetime('now', ?1)",
    [&modifier],
)?;
```
**Pros:** Consistent with all other queries, safe by construction
**Cons:** None
**Effort:** 5 minutes
**Risk:** Low

### Option 2: Compute cutoff in Rust
```rust
let cutoff = Utc::now() - chrono::Duration::days(days as i64);
self.conn.execute(
    "DELETE FROM heartbeat_sends WHERE sent_at < ?1",
    [cutoff.to_rfc3339()],
)?;
```
**Pros:** Completely avoids SQLite datetime function modifier semantics
**Cons:** Requires chrono dependency (already present)
**Effort:** 5 minutes
**Risk:** Low

## Recommended Action
Option 1 — simplest fix, maintains SQLite-native datetime handling.

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs:800-804`

## Acceptance Criteria
- [ ] `prune_old_heartbeat_sends` uses parameterized query
- [ ] No `format!()` in any SQL string in db.rs
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
**Actions:** Identified sole SQL format string interpolation in codebase
