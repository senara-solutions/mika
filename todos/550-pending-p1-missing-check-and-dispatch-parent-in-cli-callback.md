---
status: pending
priority: p1
issue_id: "550"
tags: [code-review, architecture, task-engine, teams]
dependencies: []
---

# Missing `check_and_dispatch_parent()` in CLI callback path

## Problem Statement

When `mika ask --task-id` completes a callback task, it does not call `check_and_dispatch_parent()` to check if all sibling tasks are done and fire the parent task. The server handler (`handlers.rs:458,492`) does this correctly. Without parent dispatch, team runs that suspend waiting for long-running CLI callbacks will never resume — the parent `invoke_orchestrator` task will sit in `pending` status indefinitely.

## Findings

- **Source:** Architecture strategist agent
- **Location:** `crates/mika-cli/src/commands/ask.rs` — after `run_silent_agent` completes (line 95), no parent dispatch occurs
- **Comparison:** Server handler at `crates/mika-agent/src/server/handlers.rs:458` and `:492` both call `dispatcher.check_and_dispatch_parent(&task_id)` after task completion
- **Impact:** Team suspend/resume workflow broken for CLI-based callbacks. The parent `invoke_orchestrator` task orphans.

## Proposed Solutions

### Solution A: Call `try_complete_parent_on_sibling_done` directly (Recommended)

After `run_silent_agent` completes in `ask.rs`, call `ctx.async_db.try_complete_parent_on_sibling_done(tid)`. If it returns `Some(parent_id)`, build a minimal `TaskDispatcher` and dispatch the parent. This matches the server handler's behavior.

- **Pros:** Correct behavior, team suspend/resume works via CLI
- **Cons:** Requires constructing a `TaskDispatcher` in the CLI context (needs agent lock, message sender, etc.)
- **Effort:** Medium
- **Risk:** Low

### Solution B: Rely on TaskEngine tick loop to pick up the parent

If a `TaskEngine` is running (e.g., in the TUI or server), its `check_expired_siblings` scan would eventually notice. But `mika ask` is a one-shot CLI invocation with no tick loop — so this does not apply.

- **Pros:** No code change
- **Cons:** Does not work for CLI one-shot invocations
- **Effort:** None
- **Risk:** High — parent tasks orphan

## Acceptance Criteria

- [ ] After completing a callback task via `mika ask --task-id`, parent task dispatch is triggered when all siblings are done
- [ ] Team suspend/resume works correctly when callbacks complete via CLI

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | Server handler calls check_and_dispatch_parent but CLI path does not |
