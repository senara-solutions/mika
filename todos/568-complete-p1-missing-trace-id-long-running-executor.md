---
status: complete
priority: p1
issue_id: "568"
tags: [code-review, observability, trace-id, propagation]
dependencies: []
---

# Missing trace_id propagation in long-running executor

## Problem Statement

The `skills/executor.rs` creates callback tasks for long-running exec handlers with `created_trace_id: None`, even though the `ToolContext` (which has `trace_id`) is available. This breaks the trace correlation chain for exactly the flow that benefits most from observability — long-running subprocess → callback → resume.

## Findings

- **Source:** Architecture Strategist, Agent-Native Reviewer (converged independently)
- **File:** `crates/mika-agent/src/skills/executor.rs:477`
- **Evidence:** `created_trace_id: None` when `ctx.trace_id` is in scope
- **Impact:** Callback tasks from long-running skills have no trace provenance in `unified_timeline`

## Proposed Solutions

### Option A: Pass ctx.trace_id directly (Recommended)
Change `created_trace_id: None` to `Some(ctx.trace_id.to_string())`.

- **Pros:** One-line fix, immediate correlation for long-running tasks
- **Cons:** None
- **Effort:** Small (5 min)
- **Risk:** None

## Acceptance Criteria

- [ ] `created_trace_id` is populated for callback tasks in `execute_long_running`
- [ ] Long-running task appears in `unified_timeline` with correct trace_id

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from PR #88 code review | Missed propagation site |
