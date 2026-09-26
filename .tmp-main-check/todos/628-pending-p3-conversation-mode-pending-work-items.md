---
status: pending
priority: p3
issue_id: 628
tags: [code-review, agent-native, follow-up]
dependencies: []
---

# Conversation-mode prompt does not inject pending work items

## Problem Statement

Only `SilentPromptContext` has `pending_work_items`. In conversation mode, the agent doesn't automatically know what work items are active unless it calls `list_work_items`. This creates context starvation compared to heartbeat mode.

## Findings

- **Source**: Agent-native review agent

## Proposed Solutions

### Option A: Add pending_work_items to conversation prompt (follow-up)
Inject a compact summary when active items exist.

- **Effort**: Medium
- **Risk**: Increases prompt size

## Acceptance Criteria

- [ ] Conversation-mode agent sees pending work items
