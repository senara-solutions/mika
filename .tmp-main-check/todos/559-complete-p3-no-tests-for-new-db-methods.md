---
status: complete
priority: p3
issue_id: "559"
tags: [code-review, testing]
dependencies: ["553", "554", "555"]
---

# No Tests for New DB Methods or Callback Behavior

## Problem Statement

No tests were added for: `get_undelivered_callback_tasks`, `mark_task_delivered`, `migrate_v2`, or the `is_callback_turn` behavior. The plan's acceptance criteria listed tests as required (all unchecked).

## Findings

- **Found by:** Pattern Recognition, Agent-Native Reviewer (2/8 agents)

## Proposed Solutions

Add unit tests for:
- `get_undelivered_callback_tasks` — query correctness, `since_unix` boundary, ordering
- `mark_task_delivered` — atomicity, double-claim rejection (returns false)
- `migrate_v2` — data preservation during table recreation
- `build_system_prompt` with `callback_context: Some(...)` — prompt guard output

**Effort:** Medium

## Acceptance Criteria

- [ ] Tests for `get_undelivered_callback_tasks` boundary conditions
- [ ] Tests for `mark_task_delivered` atomic claiming
- [ ] Test for prompt guard output when `callback_context` is `Some`
