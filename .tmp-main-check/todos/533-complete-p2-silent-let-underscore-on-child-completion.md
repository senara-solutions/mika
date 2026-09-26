---
status: complete
priority: p2
issue_id: 533
tags: [code-review, error-handling, agent]
dependencies: []
---

# Silent let _ = on Child Task Auto-Completion

## Problem Statement

When a team agent finishes, `update_task_completed` result is silently discarded with `let _ =`. If the DB write fails, the child task stays pending permanently, blocking the parent from firing. The team run would hang until task timeout expires.

**Severity:** P2 — Silent failure that could hang team runs.

## Findings

- `crates/mika-agent/src/agent.rs:1530-1531` — `let _ = params.db.update_task_completed(...)`
- `crates/mika-agent/src/agent.rs:1539-1542` — second instance
- Known pattern from learnings: "Never `let _ =` on database calls — use `?` or log"

## Proposed Solutions

1. **Log warning on failure**
   - `if let Err(e) = db.update_task_completed(...).await { warn!(...); }`
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] DB failures on child task completion are logged as warnings
- [ ] No silent `let _ =` on task completion calls
