---
status: complete
priority: p2
issue_id: 253
tags: [code-review, performance]
dependencies: []
---

# Parallelize Task Execution in TeamEngine

## Problem Statement

`execute_tasks()` in engine.rs runs specialist agents sequentially in a for loop. With N agents taking ~30s each, the execute phase takes N*30s. A 5-agent team could take 2.5+ minutes when the tasks are independent and could run concurrently.

## Findings

- **File:** `crates/mika-agent/src/teams/engine.rs` lines 230-264
- The `execute_tasks()` method iterates over assigned tasks sequentially, awaiting each agent invocation one at a time
- Each specialist agent performs its own LLM call (~30s) and tool usage independently
- Tasks assigned to different agents have no data dependencies between them during execution
- The workspace is shared via filesystem, and each agent has its own AsyncDatabase instance

## Proposed Solutions

Use `tokio::JoinSet` or `futures::join_all` to run independent tasks concurrently. Each agent already has its own AsyncDatabase and the workspace is shared via filesystem, so there are no shared mutable state concerns. Expected 3-5x speedup for typical team sizes.

```rust
// Instead of:
for task in &tasks {
    let result = self.run_agent(task).await?;
    // ...
}

// Use:
let mut join_set = tokio::task::JoinSet::new();
for task in tasks {
    join_set.spawn(async move {
        // run agent for task
    });
}
while let Some(result) = join_set.join_next().await {
    // collect results
}
```

## Technical Details

- Each agent invocation is independent: own AsyncDatabase, own LLM call, shared workspace via filesystem
- Need to handle error propagation from concurrent tasks (fail-fast vs collect-all)
- Consider adding a concurrency limit if teams grow large (e.g., `tokio::sync::Semaphore`)
- Workspace file conflicts are possible if two agents write to the same file; this is an edge case worth documenting

## Acceptance Criteria

- [ ] Independent tasks run concurrently using `JoinSet` or equivalent
- [ ] Total execution time is approximately equal to the longest single task, not the sum
- [ ] Errors from individual tasks are properly propagated
- [ ] Existing sequential behavior is preserved for tasks that have dependencies (if any)
- [ ] All existing tests pass

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from PR #13 code review |
| 2026-02-25 | Approved during triage session. Status: pending -> ready |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
- [tokio::task::JoinSet docs](https://docs.rs/tokio/latest/tokio/task/struct.JoinSet.html)
