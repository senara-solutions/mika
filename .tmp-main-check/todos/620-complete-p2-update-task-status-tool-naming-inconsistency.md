---
status: complete
priority: p2
issue_id: 620
tags: [code-review, naming, consistency]
dependencies: []
---

# update_task_status tool name inconsistent with work_item convention

## Problem Statement

The three work item tools use inconsistent naming: `create_work_item`, `list_work_items`, but `update_task_status`. The tool description says "work item" but the name says "task_status". This breaks the naming pattern and may confuse the agent.

## Findings

- **Source**: Pattern review agent
- **Location**: `crates/mika-agent/src/tools/update_task_status.rs`

## Proposed Solutions

### Option A: Rename to update_work_item_status (Recommended)
Rename tool, file, struct, and registration.

- **Effort**: Small (find-replace in ~3 files)
- **Risk**: Low

## Acceptance Criteria

- [ ] Tool name is `update_work_item_status`
- [ ] File renamed to `update_work_item_status.rs`
- [ ] All references updated (mod.rs, tests)
