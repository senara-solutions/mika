---
status: pending
priority: p2
issue_id: "480"
tags: [code-review, security, task-engine]
dependencies: []
---

# Unbounded action_config["text"] Forwarded to MessageSender Without Size Cap

## Problem Statement

`dispatch_send_message` extracts `config["text"]` and calls `sender.send(text)` with no length
validation. The `tasks.action_config` column has no size CHECK constraint. While the
`create_reminder` tool validates `message.len()` against `MAX_INPUT_LEN` (10,000 chars),
`ensure_recurring_task()` accepts raw `action_config: &str` with no length guard, and future
task-creation paths or direct DB manipulation could bypass the tool-level validation.
In server mode, the `GatewayMessageSender` would POST the entire unbounded payload over the network.

## Findings

- **Source**: security-sentinel review
- **Location**: `crates/mika-agent/src/task_engine/dispatcher.rs:64–75`
- `ensure_recurring_task` at `task_engine/mod.rs:24` accepts `action_config: &str` with no length guard
- The server handler at `handlers.rs:82` applies a 50,000-char check on incoming messages but
  the task dispatcher has no equivalent guard

## Proposed Solutions

### Option A: Add size cap in dispatch_send_message (Recommended)
```rust
let text = config["text"].as_str()...;
if text.len() > 50_000 {
    return Err(anyhow!("send_message text exceeds 50,000 chars"));
}
```
- **Pros**: Defensive-in-depth, consistent with server handler limit, protects all creation paths
- **Effort**: Tiny | **Risk**: None

### Option B: Add DB CHECK constraint on action_config length
Add `CHECK(length(action_config) <= 100000)` to the tasks table schema.
- **Pros**: Enforced at the DB layer
- **Cons**: Requires schema migration, only enforced on INSERT (not UPDATE)
- **Effort**: Small | **Risk**: Low

## Acceptance Criteria

- [ ] `dispatch_send_message` rejects or truncates `text` values exceeding a documented limit
- [ ] The limit is consistent with the server handler's 50,000-char check
- [ ] Error is logged, task marked failed (not silently dropped)

## Work Log

- 2026-03-06: Identified by security-sentinel review of feat/unified-task-engine
