---
status: complete
priority: p1
issue_id: "025"
tags: [code-review, security, rust-v2]
dependencies: []
---

# SQL Injection Pattern in update_commitment_status

## Problem Statement

`crates/mika-agent/src/db.rs` uses `format!` string interpolation to build SQL in `update_commitment_status`. While the interpolated value is currently a hardcoded literal, the pattern is dangerous and the `status` parameter (originating from LLM tool calls) has no allowlist validation.

**Why it matters:** Establishes a fragile pattern that could become exploitable if refactored. The LLM can write arbitrary strings into the status column.

## Findings

- **Source:** Security Sentinel (C1), Architecture Strategist (I), Performance Oracle (OPT-8)
- **Location:** `crates/mika-agent/src/db.rs:514-528`
- **Evidence:** `&format!("UPDATE commitments SET status = ?1, completed_at = {completed_at} WHERE id = ?2")`

## Proposed Solutions

### Option A: Two static SQL statements + status allowlist (Recommended)
- Validate status against `["pending", "completed", "cancelled"]`
- Use separate `conn.execute()` calls with static SQL for each branch
- **Pros:** Fully parameterized, compile-time verifiable, no format!
- **Cons:** Slight code duplication (two similar SQL strings)
- **Effort:** Small
- **Risk:** Low

### Option B: Use CASE WHEN with parameter
- Single query: `SET completed_at = CASE WHEN ?3 THEN datetime('now') ELSE NULL END`
- **Pros:** Single query, fully parameterized
- **Cons:** Slightly less readable
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria

- [ ] `status` validated against an allowlist before any SQL execution
- [ ] No `format!` used in any SQL string construction
- [ ] Invalid status returns an error
- [ ] Existing tests still pass
