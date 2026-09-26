---
title: "Agent loop max-steps fallback never follows up"
category: runtime-errors
component: crates/mika-agent/src/agent.rs
tags: [agent-loop, max-steps, continuation-turn, fallback, tool-execution]
date_identified: 2026-03-01
date_resolved: 2026-03-01
severity: high
affected_modes: [conversation]
---

# Agent Loop Max-Steps Fallback Never Follows Up

## Problem

When the agent loop exhausted its 10-step tool limit (`MAX_TOOL_STEPS`), it returned a hardcoded message: *"I need a moment to think about that. Let me get back to you."* — but never actually came back. No follow-up mechanism existed. The agent simply stopped, leaving the user stranded mid-task.

### Symptoms

- User sees "I need a moment to think about that. Let me get back to you."
- Agent stops completely — no continuation, no reminder, no follow-up
- Triggered by complex multi-step workflows (tmux automation, multi-directory file search)
- Observed twice in production on 2026-03-01 (DB messages 1252 and 1256)

### Root Cause

In `agent.rs`, `run_agent_inner` checked `result.max_steps_exceeded` and immediately returned a hardcoded fallback string without any mechanism to resume or summarize work:

```rust
if result.max_steps_exceeded {
    let fallback = "I need a moment to think about that. Let me get back to you.";
    db.save_message_with_metadata("assistant", fallback, ...).await?;
    return Ok(AgentOutput { text: Some(fallback.to_string()), .. });
}
```

The `run_loop()` function returns `max_steps_exceeded: true` after iterating `0..MAX_TOOL_STEPS` (10) without an `EndTurn` stop reason.

## Solution

Three-part fix, all scoped to `Conversation` mode only:

### Part 1: Continuation Turn

When max steps are exceeded, make one final Claude API call with tools disabled to force a text summary:

- `request.tools = None` — prevents further tool calls
- `request.thinking = None` — saves latency on a summary task
- Strip step-awareness nudge from system prompt (via saved `system_prompt_original_len`)
- Inject synthetic user message: `"[You ran out of tool steps. Summarize what you accomplished and what remains undone. Be concise.]"`
- 60-second timeout (`CONTINUATION_TIMEOUT_SECS`) — longer than `TOOL_TIMEOUT_SECS` (30s) because this is a full generation call
- Capture `usage` from continuation response for accurate token tracking

### Part 2: Step-Awareness Nudge

At step 8 of 10, append to the system prompt (conversation mode only):

```
[SYSTEM: You have 2 tool steps remaining before the limit.
Prioritize completing your current task or summarizing progress.]
```

This nudges the model to wrap up rather than continuing open-ended exploration. The nudge is stripped before the continuation turn to avoid stale context.

### Part 3: Structured Fallback

If the continuation turn fails (API error, timeout, empty response), show an honest fallback instead of the misleading "moment to think" message:

```
I ran out of steps working on that. Here's what I did:
- search_memory (done)
- run_shell (failed)
- read_file (done)

You can ask me to continue where I left off.
```

Shows last 5 tool calls with status via `format_step_exceeded_fallback()`.

## Key Design Decisions

1. **System prompt nudge vs. user message injection**: Chose system prompt to avoid role-alternation issues (after `process_tool_calls`, the last message is already a `user` role tool_result). The `follow_up_attempted` pattern uses user-message injection but occurs at a different point in the message sequence.

2. **`CONTINUATION_TIMEOUT_SECS` (60s) vs. reusing `TOOL_TIMEOUT_SECS` (30s)**: A continuation turn is a full API generation call, not a tool execution. 30s is too tight under API load. Dedicated constant with correct semantics.

3. **Strip nudge before continuation**: The nudge says "2 steps remaining" which is stale during the continuation turn. Track original system prompt length via `system_prompt_original_len` on `LoopResult` and truncate before the continuation call.

4. **Usage tracking**: Continuation response's `usage` replaces the loop's last usage (via `continuation_usage.or(result.usage)`) so token tracking is accurate.

## Prevention

- When adding hardcoded user-facing messages that imply future action ("I'll get back to you", "Let me think about that"), ensure the code actually implements the follow-up mechanism
- Agent loop safety valves (max steps, timeouts) should produce actionable output, not dead-end messages
- Test the max-steps path explicitly — it's easy to overlook because it requires 10+ consecutive tool calls

## Related Files

| File | Relevance |
|------|-----------|
| `todos/313-complete-p2-max-steps-fallback-drops-summaries.md` | Previous fix: metadata was lost on max-steps path |
| `todos/078-complete-p2-agent-loop-duplication.md` | Shared `run_loop()` extraction that this fix builds on |
| `todos/031-complete-p2-agent-loop-performance.md` | Added 5-minute total timeout wrapping the loop |
| `docs/architecture.md` Section 4 | Agent loop constants and flow documentation |
| `docs/solutions/integration-issues/mcp-client-integration-rmcp.md` | Tool dispatch chain (builtins -> skills -> MCP) |

## Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_TOOL_STEPS` | 10 | Maximum tool-use iterations per turn |
| `TOOL_TIMEOUT_SECS` | 30 | Per-tool execution timeout |
| `AGENT_TOTAL_TIMEOUT_SECS` | 300 | Total agent loop timeout (5 min) |
| `CONTINUATION_TIMEOUT_SECS` | 60 | Continuation turn timeout (new) |
