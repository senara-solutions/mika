---
status: pending
priority: p2
issue_id: "545"
tags: [code-review, data-integrity, migration]
dependencies: []
---

# migrate_persona_to_self_model can hit unique constraint violation

## Problem Statement

`migrate_persona_to_self_model` does `UPDATE core_memory SET key = 'self_model' WHERE agent_id = ?1 AND key = 'persona'`. If an agent somehow has both a `persona` and `self_model` row (e.g., partial migration, manual DB edit), this UPDATE will fail with a `UNIQUE constraint failed` on the `PRIMARY KEY (agent_id, key)`, aborting startup.

## Findings

- **Source:** Data integrity review agent
- **Location:** `crates/mika-agent/src/db.rs` line 1478-1484, called from `crates/mika-agent/src/startup.rs` line 15
- **Evidence:** The `core_memory` table has `PRIMARY KEY (agent_id, key)`. The UPDATE does not guard against the target key already existing.

## Proposed Solutions

### Option A: Guard with NOT EXISTS subquery
- **Approach:** `UPDATE core_memory SET key = 'self_model' WHERE agent_id = ?1 AND key = 'persona' AND NOT EXISTS (SELECT 1 FROM core_memory WHERE agent_id = ?1 AND key = 'self_model')`
- **Pros:** Atomic, single statement, safe
- **Cons:** Slightly more complex SQL
- **Effort:** Small
- **Risk:** Low

### Option B: Delete persona if self_model already exists
- **Approach:** In a transaction: if both exist, DELETE the persona row (self_model takes precedence since it's the new key)
- **Pros:** Handles the edge case cleanly
- **Cons:** Loses persona content (but it's obsolete)
- **Effort:** Small
- **Risk:** Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/db.rs`

## Acceptance Criteria

- [ ] Startup succeeds when agent has both `persona` and `self_model` keys
- [ ] Startup succeeds when agent has only `persona` (migrated to self_model)
- [ ] Startup succeeds when agent has only `self_model` (no-op)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | Edge case from partial migration or manual DB edits |

## Resources

- PR branch: `feat/unified-task-engine`
