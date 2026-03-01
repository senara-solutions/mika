---
title: "Fix agent loop max-steps fallback: no follow-up after 'Let me get back to you'"
type: fix
status: completed
date: 2026-03-01
---

# Fix: Agent Loop Max-Steps Fallback Never Follows Up

## Overview

When Mika's agent loop exhausts its 10-step tool limit (`MAX_TOOL_STEPS`), it returns a hardcoded message: *"I need a moment to think about that. Let me get back to you."* — but **never actually comes back**. The message is misleading: it implies a follow-up that never happens. The agent simply stops.

This was observed **twice today** (2026-03-01) in the database:
- **Message 1252:** Agent was automating tmux commands (creating a plan file + implementing features). Burned 10 steps of tmux interactions without finishing.
- **Message 1256:** Agent was searching for a skill's system prompt across multiple directories. Burned 10 steps of `find` commands without finishing.

## Root Cause

In `crates/mika-agent/src/agent.rs:666-676`:

```rust
if result.max_steps_exceeded {
    let fallback = "I need a moment to think about that. Let me get back to you.";
    let metadata = tool_calls_metadata_json(&result.tool_call_summaries);
    db.save_message_with_metadata("assistant", fallback, channel_type, metadata.as_deref())
        .await?;
    return Ok(AgentOutput {
        text: Some(fallback.to_string()),
        thinking: result.thinking,
        usage: result.usage,
    });
}
```

The `run_loop()` function (line 296-420) iterates `0..MAX_TOOL_STEPS` (10). When the loop exhausts all iterations, it returns `LoopResult { max_steps_exceeded: true }`. The caller returns the hardcoded fallback and terminates. No continuation, no reminder, no follow-up.

## Proposed Solution (Three Parts)

### Part 1: Continuation Turn (Primary Fix)

When `max_steps_exceeded` is true, instead of returning the fallback immediately, make one final Claude API call with:
- **Tools disabled** (`request.tools = None`) — forces the model to produce text, not more tool calls
- **Thinking disabled** (`request.thinking = None`) — saves latency/tokens on a summary task
- **30-second sub-timeout** — prevents the continuation from consuming the remaining 5-minute budget
- **A synthetic user message** appended: `"[You ran out of tool steps. Summarize what you accomplished and what remains undone. Be concise.]"`

This gives the user an actionable summary instead of a dead-end.

**Implementation in `run_agent_inner` (line 666-676):**

```rust
if result.max_steps_exceeded {
    // Attempt a continuation turn: tools disabled, force text summary
    request.tools = None;
    request.thinking = None;

    // Inject summarization prompt
    request.messages.push(Message {
        role: "user".to_string(),
        content: MessageContent::Text(
            "[You ran out of tool steps. Summarize what you accomplished and what remains undone. Be concise.]".to_string(),
        ),
    });

    let continuation = tokio::time::timeout(
        Duration::from_secs(TOOL_TIMEOUT_SECS),
        claude.send_message(&request),
    )
    .await;

    let text = match continuation {
        Ok(Ok(resp)) => {
            let t = resp.text();
            if t.is_empty() {
                format_step_exceeded_fallback(&result.tool_call_summaries)
            } else {
                t
            }
        }
        _ => format_step_exceeded_fallback(&result.tool_call_summaries),
    };

    let metadata = tool_calls_metadata_json(&result.tool_call_summaries);
    db.save_message_with_metadata("assistant", &text, channel_type, metadata.as_deref())
        .await?;
    return Ok(AgentOutput {
        text: Some(text),
        thinking: result.thinking,
        usage: result.usage,
    });
}
```

### Part 2: Step-Awareness Nudge

At step `MAX_TOOL_STEPS - 2` (step 8), append a note to the system prompt nudging the model to wrap up:

```
\n\n[SYSTEM: You have 2 tool steps remaining before the limit. Prioritize completing your current task or summarizing progress.]
```

**Implementation in `run_loop` (line 296):**

```rust
// Inside the for loop, before the API call at step MAX_TOOL_STEPS - 2
if mode.is_conversation() && step == MAX_TOOL_STEPS - 2 {
    if let Some(ref mut system) = request.system {
        system.push_str(
            "\n\n[SYSTEM: You have 2 tool steps remaining before the limit. \
             Prioritize completing your current task or summarizing progress.]",
        );
    }
}
```

This is injected into the system prompt (not as a user message) to avoid role-alternation issues. The nudge persists for the remaining 2 steps, which is the intended behavior.

**Gating:** Only for `Conversation` mode. Silent and team agents should not receive this nudge.

### Part 3: Honest Fallback Message

If the continuation turn fails (API error, empty response, timeout), use a structured fallback instead of the misleading "moment to think" message.

**New function `format_step_exceeded_fallback`:**

```rust
fn format_step_exceeded_fallback(summaries: &[ToolCallSummary]) -> String {
    let mut msg = String::from(
        "I ran out of steps working on that. Here's what I did:\n",
    );
    // Show last 5 tool calls max, keep it concise
    let start = summaries.len().saturating_sub(5);
    for s in &summaries[start..] {
        let status = if s.success { "done" } else { "failed" };
        msg.push_str(&format!("- {} ({})\n", s.name, status));
    }
    msg.push_str("\nYou can ask me to continue where I left off.");
    msg
}
```

## Scope

- **In scope:** `Conversation` mode only (`run_agent_inner`)
- **Out of scope:** Silent mode (result already discarded), Team mode (has its own handler — separate fix)
- **Not changing:** `MAX_TOOL_STEPS` value (10). The fix addresses the UX when the limit is hit, not whether the limit itself is correct.

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/agent.rs` | Part 1: continuation turn in `run_agent_inner` (line 666-676) |
| `crates/mika-agent/src/agent.rs` | Part 2: step-awareness nudge in `run_loop` (around line 296) |
| `crates/mika-agent/src/agent.rs` | Part 3: new `format_step_exceeded_fallback` function |

## Edge Cases

1. **Continuation turn times out** — 30s sub-timeout fires, falls to Part 3 fallback. Outer 5-minute timeout is unaffected.
2. **Continuation API returns 429/500** — Error propagated via `?`, falls to Part 3 fallback.
3. **Step-8 nudge on runs that complete at step 9** — The nudge is additive text on the system prompt. It does not prevent the agent from completing normally. No regression risk.
4. **Zero tool summaries** — Edge case where max_steps fires with no summaries (shouldn't happen but handle gracefully). Fallback says "I ran out of steps" without bullet points.
5. **Large context from 10 steps of tool results** — The continuation turn sends the full conversation. If context exceeds the model window, the API returns an error, and we fall to Part 3 fallback.
6. **Silent mode step-8 nudge** — Gated by `mode.is_conversation()`, so silent/team modes are unaffected.

## Acceptance Criteria

- [x] When agent hits 10-step limit, it makes one final API call (tools disabled) to produce a summary
- [x] Summary is saved to DB as the assistant message with tool call metadata
- [x] If continuation fails, a structured fallback with tool names is shown (not "I need a moment...")
- [x] At step 8, a nudge is injected into the system prompt (conversation mode only)
- [x] Silent and team modes are unaffected by the nudge and continuation logic
- [x] Continuation turn has a 30-second timeout
- [x] Thinking is disabled for the continuation turn
- [x] All existing agent loop tests pass
- [x] New tests cover: continuation success, continuation failure, nudge injection, mode gating

## Tests

1. `test_max_steps_continuation_produces_summary` — Mock Claude to return ToolUse for 10 steps, then EndTurn with text on the continuation turn. Verify the summary text is returned.
2. `test_max_steps_continuation_failure_uses_fallback` — Mock Claude to return ToolUse for 10 steps, then error on continuation. Verify the structured fallback is returned.
3. `test_max_steps_nudge_injected_at_step_8` — Verify the system prompt contains the nudge text after step 8.
4. `test_max_steps_nudge_not_injected_in_silent_mode` — Verify silent mode does not receive the nudge.
5. `test_format_step_exceeded_fallback` — Unit test the fallback formatter with 0, 3, and 10 summaries.
6. `test_max_steps_continuation_timeout` — Mock a slow response on continuation, verify 30s timeout fires and fallback is used.

## References

- `crates/mika-agent/src/agent.rs:27` — `MAX_TOOL_STEPS` constant
- `crates/mika-agent/src/agent.rs:296-420` — `run_loop` function
- `crates/mika-agent/src/agent.rs:470-523` — `run_agent` with total timeout
- `crates/mika-agent/src/agent.rs:526-683` — `run_agent_inner` with max_steps handling
- `todos/313-complete-p2-max-steps-fallback-drops-summaries.md` — Previous fix for metadata preservation
- `todos/031-complete-p2-agent-loop-performance.md` — Related performance concerns
