---
status: complete
priority: p2
issue_id: "059"
tags: [code-review, architecture, rust-v2]
dependencies: []
---

# Database::conn() Visibility Too Broad

## Problem Statement

`Database::conn()` is `pub` but should be `pub(crate)`. It exposes the raw SQLite connection to external crates, bypassing all Database method abstractions. External code could execute arbitrary SQL, breaking encapsulation.

**Location:** `crates/mika-agent/src/db.rs` — `pub fn conn(&self)`

**Reported by:** architecture-strategist

## Proposed Solutions

### Option A: Change to pub(crate) (Recommended)
- **Effort:** Tiny
- **Risk:** None — no external crate accesses conn() currently

## Acceptance Criteria

- [ ] `conn()` is `pub(crate)` not `pub`
- [ ] `cargo build` compiles

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from encryption-strip code review | |
