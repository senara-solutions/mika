---
status: pending
priority: p1
issue_id: "670"
tags: [code-review, architecture, telegram, delegate-task]
dependencies: []
---

# Delegate Auto-Send Causes Double Message Delivery

## Problem Statement

In `delegate_task.rs:250-262`, after `run_team_agent` completes, the delegate's text response is automatically sent to Telegram via the delegate's sender. However, the delegate agent also has `message_sender` wired into its `ToolContext`, and the updated `send_message` tool description instructs delegates to use it. This creates a double-send: the delegate sends via `send_message` during its loop AND the auto-relay sends the final text response again after the loop.

The orchestrator may also relay the result, creating a potential triple-send.

## Findings

- **Flagged by 4/6 review agents independently** (security, architecture, simplicity, agent-native)
- The `send_message` tool description was updated to instruct delegates: "When delegated a task that involves sending a message, you MUST use this tool to deliver it"
- The auto-relay at line 254 sends the delegate's final text response (LLM's last assistant message), which may duplicate what `send_message` already sent
- User explicitly accepted this trade-off in brainstorm but all reviewers flagged it

## Proposed Solutions

### Option A: Remove auto-send relay (Recommended)
- Remove lines 225 (`delegate_sender_for_relay` clone) and 254-262 (auto-send block)
- Rely on delegate using `send_message` tool for Telegram delivery
- Orchestrator gets the result via tool output for its own use
- **Pros:** Eliminates duplication, ~10 LOC removed, cleaner design
- **Cons:** If delegate doesn't call `send_message`, user gets no Telegram message from the delegate (only orchestrator's relay)
- **Effort:** Small
- **Risk:** Low

### Option B: Track whether delegate used send_message
- Add a counter/flag in ToolContext tracking `send_message` calls
- Only auto-relay if the delegate did NOT call `send_message` during its run
- **Pros:** Handles both cases correctly
- **Cons:** Adds complexity, couples delegate_task to send_message internals
- **Effort:** Medium
- **Risk:** Medium (coupling)

### Option C: Keep as-is, document trade-off
- Accept double-send as a feature (user sees both `[mika-dev] hello` and orchestrator relay)
- Document in CLAUDE.md that delegate text responses are auto-relayed
- **Pros:** No code change
- **Cons:** Users receive duplicate messages
- **Effort:** None
- **Risk:** Poor UX

## Recommended Action

_To be decided during triage_

## Technical Details

- **Affected files:** `crates/mika-agent/src/tools/delegate_task.rs`
- **Lines:** 225, 250-266

## Acceptance Criteria

- [ ] Delegate agent sending a message via `send_message` tool does NOT produce a duplicate on Telegram
- [ ] Delegate agent completing without calling `send_message` still returns result to orchestrator
- [ ] All existing delegate_task tests pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-15 | Identified during code review | 4/6 agents flagged independently |

## Resources

- PR #157: feat/149/multi-agent-telegram-delivery
- Brainstorm: docs/brainstorms/2026-03-15-telegram-prefix-attribution-brainstorm.md
