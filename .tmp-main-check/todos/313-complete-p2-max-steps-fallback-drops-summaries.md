---
status: complete
priority: p2
issue_id: "313"
tags: [code-review, correctness, agent-native]
dependencies: []
---

# max_steps_exceeded fallback drops accumulated tool summaries

## Problem Statement

When the agent hits `MAX_TOOL_STEPS` (10 steps), the fallback message is saved via `db.save_message()` without metadata, even though `result.tool_call_summaries` contains summaries from all the tools that ran before the limit was hit. This means the next turn gets zero context about what was accomplished.

Identified by: architecture-strategist, agent-native-reviewer

## Findings

- `run_agent_inner` line 566-568 uses `save_message` (no metadata) for the max-steps fallback
- `result.tool_call_summaries` is populated but dropped
- This is exactly the scenario where introspection is most useful — the agent needs to know what it already did

## Proposed Solutions

### Option A: Use save_message_with_metadata in fallback (Recommended)
```rust
if result.max_steps_exceeded {
    let fallback = "I need a moment to think about that. Let me get back to you.";
    let metadata = tool_calls_metadata_json(&result.tool_call_summaries);
    db.save_message_with_metadata("assistant", fallback, channel_type, metadata.as_deref()).await?;
```
- Pros: Preserves tool context, removes the need for `#[allow(dead_code)]` on LoopResult field
- Cons: None
- Effort: Small

## Technical Details

- **Affected file:** `crates/mika-agent/src/agent.rs:566-568`
- **Related:** Also removes the need for `#[allow(dead_code)]` on `tool_call_summaries` field in `LoopResult`

## Acceptance Criteria

- [ ] Max-steps fallback saves metadata with tool summaries
- [ ] `#[allow(dead_code)]` removed from `tool_call_summaries` field if it becomes used

## Work Log

- 2026-02-27: Identified during code review of commit 573596b
