---
status: pending
priority: p2
issue_id: "503"
tags: [code-review, architecture, reliability, cli]
dependencies: []
---

# `mika ask --task-id` Marks Task Complete Before Running Agent — Non-Retriable on Failure

## Problem Statement

The `mika ask --task-id` CLI path calls `update_task_completed` BEFORE `run_agent`. If `run_agent` fails (API error, timeout, kill signal), the task is permanently `completed` with no recovery path. The callback result is silently lost and the agent is never resumed.

## Findings

- **Source**: architecture-strategist (F-4 Medium)
- **Location**: `crates/mika-cli/src/commands/ask.rs:61-72`

```rust
ctx.async_db.update_task_completed(tid, Some(&user_message)).await?;
// ...
let output = agent::run_agent(&AgentParams { ... }).await?;
```

The task is committed as `completed` before the agent run starts. If `run_agent` errors or the process is killed, the task stays `completed` but the agent was never resumed. The contract of `callback` tasks — result delivered to agent — is silently broken.

This differs from the HTTP handler (`POST /tasks/{id}/complete`) which at least has the task marked before spawning, but there the spawn is async and failure is logged. The CLI path is worse: it errors out with `?` after the DB write, leaving no log that the agent never ran.

## Proposed Solutions

### Option A: Run agent first, mark complete on success (Recommended)

```rust
// Run the agent with the task result as context
let output = agent::run_agent(&AgentParams { ... }).await?;

// Only mark completed if agent succeeded
ctx.async_db.update_task_completed(tid, Some(&user_message)).await?;
```

This is safe in the CLI context (single-process, synchronous). If the agent fails, the task remains `pending` and can be retried by re-running `mika ask --task-id`.

- **Effort**: Tiny | **Risk**: Low

### Option B: Keep current ordering, add explicit error handling and log

```rust
ctx.async_db.update_task_completed(tid, Some(&user_message)).await?;
if let Err(e) = agent::run_agent(...).await {
    eprintln!("WARNING: task {} marked complete but agent run failed: {}", tid, e);
    // Task cannot be retried — document this limitation
}
```

Documents the limitation explicitly but does not fix the retryability issue.

- **Effort**: Tiny | **Risk**: None (cosmetic only)

## Acceptance Criteria

- [ ] `mika ask --task-id` runs the agent BEFORE marking the task complete
- [ ] If the agent run fails, the task remains in `pending` state and can be retried
- [ ] Existing tests for `--task-id` path pass

## Work Log

- 2026-03-06: Identified by architecture-strategist review of feat/unified-task-engine
