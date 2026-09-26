---
status: pending
priority: p3
issue_id: 626
tags: [code-review, yagni, simplification]
dependencies: []
---

# --parent-task-id CLI flag is YAGNI (no relay consumer exists)

## Problem Statement

The `--parent-task-id` flag prepends `[work-item:{uuid}]` to user messages with no parsing, no validation, and no consumer. The claude-asked relay that would use this does not exist yet. The flag also lacks UUID format validation.

## Findings

- **Source**: Simplicity review agent, Security review agent

## Proposed Solutions

### Option A: Remove flag, add when relay ships
- **Effort**: Small (3 CLI files)

### Option B: Keep but add UUID validation
- **Effort**: Small

## Acceptance Criteria

- [ ] Flag removed or properly validated
