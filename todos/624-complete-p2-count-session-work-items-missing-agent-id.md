---
status: complete
priority: p2
issue_id: 624
tags: [code-review, security, defense-in-depth]
dependencies: []
---

# count_session_work_items missing agent_id scope

## Problem Statement

`count_session_work_items` query lacks `AND agent_id = ?2`. In current single-agent-per-container architecture this is safe, but for defense-in-depth the query should be scoped.

## Findings

- **Source**: Security review agent
- **Location**: `crates/mika-agent/src/db.rs` — `count_session_work_items`

## Proposed Solutions

### Option A: Add agent_id filter (Recommended)
- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] Query includes `AND agent_id = ?2`
