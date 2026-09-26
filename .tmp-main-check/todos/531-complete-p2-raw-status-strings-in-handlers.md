---
status: complete
priority: p2
issue_id: 531
tags: [code-review, consistency, server]
dependencies: []
---

# Raw Status String Literals in handlers.rs Instead of Constants

## Problem Statement

The new code in `server/handlers.rs` uses raw string literals `"pending"`, `"in_progress"`, `"failed"` instead of the `task_status::PENDING`, `task_status::FAILED` constants defined in `task_engine/types.rs`. This violates the project convention "Using constants instead of bare string literals prevents silent typos."

**Severity:** P2 — Inconsistency that risks silent typos.

## Findings

- `crates/mika-agent/src/server/handlers.rs` lines 364, 378, 413, 425 — raw strings
- `crates/mika-agent/src/task_engine/types.rs` — defines `task_status::PENDING`, etc.

## Proposed Solutions

1. **Replace all raw strings with constants**
   - Import `task_engine::types::task_status` and use the constants
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] All status string literals in handlers.rs replaced with task_status constants
