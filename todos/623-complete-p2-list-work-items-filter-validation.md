---
status: complete
priority: p2
issue_id: 623
tags: [code-review, input-validation, consistency]
dependencies: []
---

# list_work_items does not validate status and source filter values

## Problem Statement

`list_work_items` passes `status` and `source` filters directly to `list_manual_tasks` without validating against known values. Invalid filters silently return empty results, potentially masking real data. `update_task_status` validates status but `list_work_items` does not.

## Findings

- **Source**: Security review agent, Pattern review agent

## Proposed Solutions

### Option A: Add validation (Recommended)
Validate `status` against `VALID_STATUSES` and `source` against `VALID_SOURCES`, returning errors for unknown values.

- **Effort**: Small
- **Risk**: None

## Acceptance Criteria

- [ ] Invalid status/source filters return error messages
- [ ] Tests for invalid filter values
