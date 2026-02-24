---
status: pending
priority: p3
issue_id: "071"
tags: [code-review, quality, rust-v2]
dependencies: ["063"]
---

# update_fact Does Not Capture before_value in Audit Log

## Problem Statement

`update_fact` passes `None` for `before_value` when logging the memory event. `update_core_memory` correctly captures the before snapshot. This makes it impossible to audit what the commitment's previous status was.

## Findings

- **Source:** agent-native-reviewer
- **Location:** `crates/mika-agent/src/tools/update_fact.rs:94-97`
- **Evidence:** `before_value: None` while `update_core_memory.rs:109,173-174` captures before state

## Proposed Solutions

### Option A: Query current status before update (Recommended)
- Query commitment's current status before calling `update_commitment_status`
- Pass previous status as `before_value` in audit log
- **Pros:** Audit completeness parity with update_core_memory
- **Cons:** One extra SELECT per update (negligible)
- **Effort:** Small
- **Risk:** None
- **Note:** Pairs well with #063 (if Option B is chosen there, the SELECT serves both purposes)

## Acceptance Criteria

- [ ] `before_value` populated in memory event for commitment status updates
- [ ] Test verifies audit log includes previous status
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review of commit 3619d13 | Audit logs should capture before/after for all mutations |
