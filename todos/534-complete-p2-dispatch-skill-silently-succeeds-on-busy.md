---
status: complete
priority: p2
issue_id: 534
tags: [code-review, logic-error, task-engine]
dependencies: [526]
---

# dispatch_skill_by_name Silently Succeeds When Agent Is Busy

## Problem Statement

`dispatch_skill_by_name` returns `Ok(())` when `try_lock()` fails on the agent mutex. This means the skill task is marked as completed without ever running. `dispatch_resume_agent` correctly returns `Err(...)` for the same condition, triggering re-queue.

**Severity:** P2 — Skill tasks silently lost when agent is busy.

## Findings

- `crates/mika-agent/src/task_engine/dispatcher.rs:149-150` — `Ok(())` on busy
- `crates/mika-agent/src/task_engine/dispatcher.rs:204-205` — `Err(anyhow!(...))` on busy (correct)

## Proposed Solutions

1. **Return Err on agent busy, same as dispatch_resume_agent**
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] dispatch_skill_by_name returns Err when agent is busy
- [ ] Skill tasks get re-queued, not silently dropped
