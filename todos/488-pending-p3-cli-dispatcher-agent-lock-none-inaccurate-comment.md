---
status: pending
priority: p3
issue_id: "488"
tags: [code-review, quality, task-engine]
dependencies: []
---

# CLI TaskDispatcher agent_lock: None Allows Concurrent Claude API Calls

## Problem Statement

In the CLI's `chat.rs`, `TaskDispatcher` is created with `agent_lock: None`. The comment says
"CLI serializes via channel, no lock needed." This is inaccurate: the channel serializes user
messages to the agent loop but does not prevent the `TaskEngine` tick loop from dispatching a
heartbeat/reflection silent agent turn concurrently in its own `tokio::spawn`ed task. With
`agent_lock: None`, the heartbeat's `dispatch_heartbeat` skips the `try_lock` pre-filter guard.
Concurrent Claude API calls from a heartbeat and a user message can cause unnecessary 429
rate-limit responses.

## Findings

- **Source**: architecture-strategist review
- **Location**: `crates/mika-cli/src/commands/chat.rs` (TaskDispatcher construction with agent_lock: None)
- `dispatcher.rs:134–144` — `agent_lock` guard only acquired when `self.agent_lock.is_some()`
- Not architecturally unsafe (DB thread serializes all DB ops), but causes unnecessary
  concurrent LLM API calls under heartbeat conditions

## Proposed Solutions

### Option A: Pass agent_lock from CLI context to CLI TaskDispatcher
The CLI already has an `agent_lock` (or equivalent) for serializing the main agent loop.
Pass it to the `TaskDispatcher` so heartbeat defers when user message is in-flight.
- **Effort**: Small | **Risk**: Low

### Option B: Fix the inaccurate comment
Update the comment to accurately describe why concurrent calls are acceptable in CLI mode,
if they are intentionally accepted.
- **Effort**: Tiny | **Risk**: None

## Acceptance Criteria

- [ ] Either CLI dispatcher uses `agent_lock` to prevent concurrent Claude calls during heartbeat
- [ ] OR comment accurately documents the known concurrent-call behavior and why it's acceptable

## Work Log

- 2026-03-06: Identified by architecture-strategist review of feat/unified-task-engine
