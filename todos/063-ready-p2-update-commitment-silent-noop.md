---
status: ready
priority: p2
issue_id: "063"
tags: [code-review, quality, rust-v2]
dependencies: []
---

# update_commitment_status Silently Succeeds for Non-Existent IDs

## Problem Statement

`Database::update_commitment_status()` executes an UPDATE and returns `Ok(())` regardless of whether the target row exists. The `update_fact` tool then tells the agent "Updated commitment (id:X) status to 'completed'" even when no row was affected. This causes the agent to give the user false confirmation.

**Why it matters:** The user asks "mark my task done", the agent says "Done!" but nothing actually changed in the database.

## Findings

- **Source:** architecture-strategist, security-sentinel, agent-native-reviewer
- **Location:** `crates/mika-agent/src/db.rs:460-478`, `crates/mika-agent/src/tools/update_fact.rs:94`
- **Evidence:** `rusqlite::Connection::execute()` returns the number of affected rows, but the return value is ignored

## Proposed Solutions

### Option A: Return affected row count from DB method (Recommended)
- Change `update_commitment_status` to return `Result<bool>` (true if row existed)
- Have `update_fact` check the return and report "Commitment not found" on false
- **Pros:** Simple, idiomatic, no extra query
- **Cons:** Minor signature change
- **Effort:** Small
- **Risk:** None

### Option B: Query before update
- SELECT the commitment first, then UPDATE
- **Pros:** Can capture before_value for audit log
- **Cons:** Extra query, TOCTOU race (negligible for single-user SQLite)
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] `update_commitment_status` returns whether a row was affected
- [ ] `update_fact` returns error message when commitment ID not found
- [ ] Test for nonexistent ID case added
- [ ] All existing tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review of commit 3619d13 | rusqlite execute() returns usize row count |
