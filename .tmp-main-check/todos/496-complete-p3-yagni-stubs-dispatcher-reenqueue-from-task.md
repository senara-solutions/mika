---
status: complete
priority: p3
issue_id: "496"
tags: [code-review, quality, task-engine]
dependencies: []
---

# YAGNI: Dispatcher Stubs, ReEnqueue Struct, and from_task Dead Code

## Problem Statement

Several YAGNI violations were found in the new task_engine code:

1. **`dispatch_resume_agent` and `dispatch_invoke_orchestrator` stubs** — Both return `Err("not yet implemented")`, identical to the `unknown action_type` fallback. They add ~40 lines of boilerplate and two dead constants (`RESUME_AGENT`, `INVOKE_ORCHESTRATOR`) for Phase 4 features.

2. **`ReEnqueue` struct** in `engine.rs` — Duplicates all 5 fields of `QueuedTask`. The re-enqueue channel type could simply be `mpsc::channel::<QueuedTask>(64)`, eliminating `ReEnqueue`.

3. **`QueuedTask::from_task`** in `queue.rs` — Defined but never called. The engine constructs `QueuedTask` values inline.

## Findings

- **Source**: code-simplicity-reviewer review
- **Locations**:
  - `dispatcher.rs:80–83, 327–334` (stubs), `dispatcher.rs:466–492` (test for stubs)
  - `types.rs:23–24` (RESUME_AGENT, INVOKE_ORCHESTRATOR constants)
  - `engine.rs:25–31` (ReEnqueue struct)
  - `queue.rs:36–46` (from_task)
- Total estimated LOC reduction: ~80 lines

## Proposed Solutions

### Option A: Remove all YAGNI items (Recommended)
- Remove `dispatch_resume_agent`, `dispatch_invoke_orchestrator`, their doc comments, and the test
- Remove `RESUME_AGENT` and `INVOKE_ORCHESTRATOR` from `types.rs`
- Replace `ReEnqueue` channel type with `QueuedTask` directly
- Remove `QueuedTask::from_task`
When Phase 4 arrives, re-adding these takes minutes.
- **Effort**: Small | **Risk**: None (dead code removal)

### Option B: Mark with explicit TODO comments
Add `// TODO(Phase 4): implement` comments but keep the stubs.
- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] Stubs removed (or kept with explicit future-phase documentation)
- [ ] `ReEnqueue` struct replaced with `QueuedTask` in channel type
- [ ] `from_task` removed (no callers)
- [ ] `cargo test` passes, `cargo clippy` passes with no new warnings

## Work Log

- 2026-03-06: Identified by code-simplicity-reviewer review of feat/unified-task-engine
