---
status: complete
priority: p1
issue_id: 524
tags: [code-review, logic-error, teams, task-engine]
dependencies: []
---

# Race Condition: Task Tree Fires Even When Team Run Completes Synchronously

## Problem Statement

Every `execute_tasks()` call creates a parent `invoke_orchestrator` task plus N child `resume_agent` tasks, even for synchronous team runs. When all agents complete synchronously:

1. Child tasks are auto-completed on agent turn end
2. `try_complete_parent_on_sibling_done` fires the parent (claims it from pending → in_progress)
3. The parent dispatches `invoke_orchestrator`, which calls `resume_team_run`
4. Meanwhile, the original `execute_inner()` flow is already continuing with review/deliver

This creates a race where both the synchronous flow and the async dispatcher try to continue the team run simultaneously, potentially causing duplicate review/deliver phases or DB conflicts.

**Severity:** P1 — Race condition that could cause duplicate team run completions.

## Findings

- `crates/mika-agent/src/teams/engine.rs` — task tree always created in `execute_tasks()`
- `crates/mika-agent/src/task_engine/dispatcher.rs` — `dispatch_invoke_orchestrator` has no guard checking if team run is still suspended
- The parent task transitions pending → in_progress via sibling completion even when team run is completing normally

## Proposed Solutions

1. **Guard dispatch_invoke_orchestrator with team run status check**
   - Before proceeding, verify team run is in `suspended` status
   - If not suspended (already running/completed), skip dispatch and mark parent task completed
   - Pros: Simple guard, minimal change
   - Cons: Orphaned task tree still exists in DB
   - Effort: Small
   - Risk: Low

2. **Only create task tree when suspension is detected**
   - Move task tree creation to after agents complete, only if `pending_grandchildren > 0`
   - Pros: No orphaned tasks for sync runs
   - Cons: Larger refactor, child tasks need to be created retroactively
   - Effort: Medium
   - Risk: Medium

3. **Cancel parent task when execute_tasks returns false (not suspended)**
   - After synchronous completion, cancel the invoke_orchestrator parent task
   - Pros: Clean, explicit
   - Cons: Still creates unnecessary tasks
   - Effort: Small
   - Risk: Low

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/dispatcher.rs`, `crates/mika-agent/src/teams/engine.rs`
- **Components:** Task engine, team engine, dispatcher

## Acceptance Criteria

- [ ] Synchronous team runs do not trigger dispatch_invoke_orchestrator
- [ ] Async team runs (with long_running tools) still correctly suspend and resume
- [ ] No duplicate review/deliver phases observed
