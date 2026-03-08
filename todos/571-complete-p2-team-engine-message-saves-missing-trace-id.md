---
status: complete
priority: p2
issue_id: "571"
tags: [code-review, observability, trace-id, team-engine]
dependencies: []
---

# Team engine agent response/error messages pass None for trace_id

## Problem Statement

In `teams/engine.rs`, the `execute_tasks` method spawns tasks via JoinSet. When agent responses/errors are saved to the messages table (lines 870-906), they pass `None` for trace_id. The `self.trace_id` is not accessible inside the spawned task closure.

## Findings

- **Source:** Architecture Strategist, Data Integrity Guardian, Agent-Native Reviewer
- **File:** `crates/mika-agent/src/teams/engine.rs:875` and `:904`
- **Evidence:** `save_message_with_metadata(..., None)` inside JoinSet task
- **Impact:** Team agent response messages have NULL trace_id in unified_timeline

## Proposed Solutions

### Option A: Clone trace_id into spawned task (Recommended)
Clone `self.trace_id` into the JoinSet closure alongside existing clones (`team_db`, `run_id`, etc.).

- **Pros:** Simple fix, follows existing clone pattern
- **Cons:** One extra String clone per spawned agent task
- **Effort:** Small (10 min)
- **Risk:** None

### Option B: Return trace_id from run_team_agent
Have `run_team_agent` return a struct with both response and trace_id. Use the delegated agent's trace_id instead of the orchestrator's.

- **Pros:** Semantically correct (uses agent's own trace_id)
- **Cons:** Larger refactor, changes return type
- **Effort:** Medium
- **Risk:** Low

## Acceptance Criteria

- [ ] Agent response and error messages saved with non-NULL trace_id
- [ ] `cargo test` passes

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from PR #88 code review | Spawned task lacks access to self |
