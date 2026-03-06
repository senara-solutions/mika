---
status: pending
priority: p2
issue_id: "469"
tags: [code-review, correctness, task-engine]
dependencies: [459]
---

# 469 · Stub dispatchers return `Ok(())` — unimplemented actions silently marked `completed`

## Problem Statement

`dispatch_resume_agent` and `dispatch_invoke_orchestrator` emit a `warn!`
log but return `Ok(())`. A task with `action_type = "resume_agent"` or
`"invoke_orchestrator"` will be marked `completed` (via `update_task_completed`)
with no execution, silently disappearing. These are valid action types per
the DB schema CHECK constraint, so tasks can legitimately reach these handlers.

## Findings

- **Location:** `crates/mika-agent/src/task_engine/dispatcher.rs:78–80`, `318–324`
- Schema at `crates/mika-agent/src/db.rs:397–398` validates these action types
- After todo 459 (add `update_task_failed`), the fix is to return `Err` instead of `Ok(())`

## Proposed Solutions

### Option A — Return `Err` for unimplemented actions (recommended)
```rust
fn dispatch_resume_agent(&self, task_id: &str) -> Result<()> {
    warn!(task_id, "resume_agent not yet implemented");
    Err(anyhow!("resume_agent action type not yet implemented"))
}
```
The task will be marked `failed` with the error string, making the state visible.

**Effort:** Trivial | **Risk:** Low

### Option B — Keep `Ok(())` but add TODO comment
Document intent clearly; treat as a soft skip.
**Cons:** Task disappears silently.

## Recommended Action

Option A. Unimplemented features should fail loudly.

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/dispatcher.rs`
- Depends on todo 459 for `update_task_failed`

## Acceptance Criteria

- [ ] `dispatch_resume_agent` returns `Err` with "not yet implemented"
- [ ] `dispatch_invoke_orchestrator` returns `Err` with "not yet implemented"
- [ ] Tasks with these action types appear as `failed` in DB after dispatch attempt

## Work Log

- 2026-03-06: Identified by code quality review agent (QUAL-3)
