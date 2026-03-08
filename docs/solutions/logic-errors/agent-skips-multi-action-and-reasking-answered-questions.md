---
title: "Agent processes only one action from multi-part requests and re-asks answered questions"
date: "2026-03-08"
module: "prompt"
severity: "medium"
tags:
  - "system-prompt"
  - "conversation-behavior"
  - "multi-action"
  - "conversation-continuity"
  - "prompt-engineering"
related_files:
  - "crates/mika-agent/src/prompt.rs"
---

# Agent Skips Multi-Action Requests and Re-Asks Answered Questions

## Problem Statement

Two related conversation behavior defects observed in production:

1. **Single-action processing:** When a user asks to update/create multiple items in one message (e.g., "update both reminders"), the agent only processes one and asks about the other in a follow-up message — even though it has all the information needed.

2. **Re-asking answered questions:** When the user has already answered a question (e.g., "yes please" to a confirmation about a black bin reminder), the agent ignores that answer and re-asks the same question in the next turn.

## Root Cause

The system prompt's Tool Usage section (`prompt.rs`) lacked behavioral guidance for these two scenarios:

1. **No multi-action instruction.** The agent defaults to a conservative one-at-a-time pattern because it was never told to batch similar operations. The tool execution loop already supports multiple tool calls per turn (max 10 steps), but the LLM didn't know to use this capability.

2. **Passive memory instruction.** The only relevant instruction was:
   ```
   Use search_memory to find stored facts before asking the user to repeat information.
   ```
   This is passive ("use search_memory") rather than mandatory, and only covers stored facts — not the conversation history already present in the LLM's context window (last 20 messages are always loaded).

## Solution

Added two behavioral instructions to the `## Tool Usage` section in `build_system_prompt()`:

### 1. Conversation continuity (strengthened existing instruction)

**Before:**
```
- Use search_memory to find stored facts before asking the user to repeat information.
```

**After:**
```
- Before asking a clarifying question, check the conversation history —
  the user may have already answered it in a previous message.
  Never re-ask something the user already told you.
  Also use search_memory to find stored facts before asking the user to repeat information.
```

### 2. Multi-action batching (new instruction)

```
- When the user asks you to do multiple things in one message
  (e.g. "update both reminders", "create tasks for X and Y"),
  handle ALL of them in the same turn. Use multiple tool calls —
  do not process one and ask about the rest. If you have enough
  information for all actions, execute them all.
```

## Key Design Decisions

- **Prompt-only fix.** No architectural changes needed. The tool execution loop already processes multiple `ToolUse` blocks sequentially in a single turn. The issue was purely that the LLM wasn't instructed to use this capability.

- **"If you have enough information" escape clause.** The multi-action instruction includes this qualifier so the agent doesn't blindly batch when the user's intent is genuinely ambiguous.

- **Silent mode not affected.** These instructions are only in `build_system_prompt()` (conversational mode), not `build_silent_prompt()`. Silent mode is for background tasks (heartbeat, reminders, reflection) where multi-action batching and conversation continuity are not applicable.

- **Token budget impact.** Adds ~80 tokens to the system prompt. Negligible given the prompt is already 2000+ tokens.

## Prevention Strategies

When adding new agent capabilities or tools:

1. **Always add behavioral guidance in the system prompt.** Supporting a capability in code (e.g., multiple tool calls per turn) is not enough — the LLM needs explicit instruction to use it.

2. **Use active, mandatory language.** "Never re-ask" is stronger than "Use search_memory to avoid re-asking." Passive suggestions get ignored under complex reasoning.

3. **Test with realistic multi-turn scenarios.** Single-turn tests won't catch conversation continuity issues. Consider testing with conversation histories that include prior user answers.

## Related

- `docs/solutions/integration-issues/adding-prompt-only-bundled-skill.md` — Pattern for prompt-only changes
- `crates/mika-agent/src/prompt.rs:305-320` — Location of the fix
