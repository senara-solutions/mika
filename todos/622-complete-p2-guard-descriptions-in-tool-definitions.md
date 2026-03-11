---
status: complete
priority: p2
issue_id: 622
tags: [code-review, agent-native, ux]
dependencies: []
---

# Tool descriptions don't mention guards, wasting agent tool steps

## Problem Statement

`create_work_item` description doesn't mention callback-turn guard, task-context guard, depth cap, or session cap. `update_task_status` doesn't mention it only works on manual tasks. The agent discovers guards via error messages after failed attempts, wasting tool steps.

## Findings

- **Source**: Agent-native review agent

## Proposed Solutions

### Option A: Add guard summaries to descriptions (Recommended)
- `create_work_item`: Add "Cannot be used during callback turns. Max 5 agent-created items per session. Max nesting depth 3."
- `update_task_status`: Add "Only works on manual work items, not system tasks."

- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] Guard limits mentioned in tool descriptions
- [ ] Agent can anticipate failures without trial-and-error
