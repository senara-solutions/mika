---
status: pending
priority: p3
issue_id: 627
tags: [code-review, agent-native, follow-up]
dependencies: []
---

# Missing get_work_item tool for single-item inspection

## Problem Statement

The agent has no tool to inspect a single work item's full state (creation date, reference URL, source, parent, children, audit trail). This breaks the "query before write" convention. The existing `get_task` tool could potentially serve this purpose but its output format may not surface work-item-specific fields.

## Findings

- **Source**: Agent-native review agent

## Proposed Solutions

### Option A: Extend get_task to show work-item fields (Recommended)
The existing `get_task` tool already returns task details. Verify it shows reference_url and source for manual tasks.

### Option B: Create dedicated get_work_item tool
- **Effort**: Medium

## Acceptance Criteria

- [ ] Agent can inspect full work item details by ID
