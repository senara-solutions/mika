---
status: pending
priority: p2
issue_id: "043"
tags: [code-review, agent-native, tools, rust-v2]
dependencies: []
---

# No Tool for Updating Commitment Status

## Problem Statement
The agent can create commitments via `store_fact` but cannot mark them as completed or cancelled. The DB method `update_commitment_status(id, status)` exists at db.rs:673 but no tool wraps it. This breaks the core workflow: the agent tracks commitments but can never close the loop.

**Location:** `crates/mika-agent/src/db.rs:673` (DB method exists), no tool wrapper

**Reported by:** agent-native-reviewer

## Proposed Solutions

### Option A: Add update_fact tool (Recommended)
Create an `update_fact` tool that can update commitment status (and potentially other fact types).
- **Pros:** Clean agent-native interface, extensible to other updates
- **Cons:** New tool adds to Claude's tool list
- **Effort:** Small
- **Risk:** Low

### Option B: Add status update to store_fact
Extend `store_fact` with an optional `id` parameter that triggers an update instead of insert.
- **Pros:** Fewer tools
- **Cons:** Overloaded semantics
- **Effort:** Small
- **Risk:** Low

## Acceptance Criteria
- [ ] Agent can mark a commitment as "completed" or "cancelled"
- [ ] Audit log records the status change
- [ ] Test: create commitment, update status, verify

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | |
