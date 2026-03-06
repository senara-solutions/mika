---
status: complete
priority: p2
issue_id: "508"
tags: [code-review, security, resource-management, validation]
dependencies: []
---

# `timeout_secs` Has No Upper Bound — Callback Tasks Can Accumulate Indefinitely

## Problem Statement

`create_task` accepts any positive `i64` for `timeout_secs`, including `i64::MAX` (year 292 billion). Callback tasks with no timeout and no count limit will accumulate in the DB indefinitely if the external caller never delivers a result. There is also no cap on the number of recurring tasks an agent can create.

## Findings

- **Source**: security-sentinel (F-4 Medium)
- **Location**: `crates/mika-agent/src/tools/create_task.rs:173-183`

```rust
let timeout_at = if let Some(secs) = input["timeout_secs"].as_i64() {
    if secs > 0 {
        Some(Utc::now().timestamp() + secs)
    } else {
        return Ok(ToolOutput::error("'timeout_secs' must be a positive integer."));
    }
} else {
    None
};
```

Any positive `i64` is accepted. Setting `timeout_secs = 9223372036854775807` produces `timeout_at` in year 292 billion. `startup_recovery` would never expire such a task.

Additionally: cron expressions are validated for parse success but there is no limit on how many recurring tasks an agent can create. An agent (or prompt-injected instruction) could create thousands of `recurring` tasks firing every minute.

## Proposed Solutions

### Option A: Cap `timeout_secs` + add max-task guard (Recommended)

```rust
const MAX_TIMEOUT_SECS: i64 = 7_776_000; // 90 days

if secs > MAX_TIMEOUT_SECS {
    return Ok(ToolOutput::error(
        format!("'timeout_secs' cannot exceed {} seconds (90 days).", MAX_TIMEOUT_SECS)
    ));
}
```

Additionally, in `create_task` before inserting:
```rust
let active_count = ctx.db.count_active_tasks().await?;
if active_count >= MAX_ACTIVE_TASKS {
    return Ok(ToolOutput::error("Maximum active task limit reached. Cancel existing tasks first."));
}
```

Where `MAX_ACTIVE_TASKS = 50` (reasonable upper bound for a personal AI assistant).

- **Effort**: Small | **Risk**: None

### Option B: Cap `timeout_secs` only

Skip the active-task count guard. Still prevents infinite-future timeouts.

- **Effort**: Tiny | **Risk**: No protection against task flooding

## Acceptance Criteria

- [ ] `timeout_secs` is capped at 90 days (7,776,000 seconds) with a clear error message
- [ ] `create_task` rejects requests when the agent already has too many active tasks
- [ ] Existing `create_task` tests pass
- [ ] New test: `timeout_secs = i64::MAX` returns an error

## Work Log

- 2026-03-06: Identified by security-sentinel review of feat/unified-task-engine
