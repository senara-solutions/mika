---
title: "fix: Multi-action batching and conversation continuity in system prompt"
type: fix
status: completed
date: 2026-03-08
---

# fix: Multi-action batching and conversation continuity in system prompt

## Overview

The agent has two related conversation behavior defects:

1. **Single-action processing:** When a user asks to update/create multiple items in one message (e.g., "update both reminders"), the agent only processes one and asks about the other — even though it has all the information needed and the tool loop supports multiple tool calls per turn.

2. **Re-asking answered questions:** When the user has already answered a question (e.g., "yes please" to a confirmation), the agent ignores that answer and re-asks the same question in the next turn.

Both issues stem from missing behavioral guidance in the system prompt. The agent infrastructure (multi-tool-call support, conversation history access) already works correctly.

## Problem Statement

### Problem 1: No multi-action guidance

The system prompt's Tool Usage section (`prompt.rs:295-373`) contains no instruction about handling multiple related requests in a single turn. The agent defaults to a conservative one-at-a-time pattern: process one item, then ask about the next.

**Current prompt (line 306):**
```
"Use search_memory to find stored facts before asking the user to repeat information."
```

This only covers stored facts, not multi-action batching from the current message.

### Problem 2: Passive conversation continuity

The `search_memory` instruction is passive ("use search_memory") rather than mandatory. It doesn't tell the agent to check the recent conversation history (which is already loaded — last 20 messages) before asking clarifying questions. The agent effectively "skips" user answers that are right there in the conversation window.

## Proposed Solution

Add two new instructions to the `## Tool Usage` section in `build_system_prompt()` (`crates/mika-agent/src/prompt.rs`), and strengthen the existing `search_memory` instruction.

### Change 1: Multi-action batching instruction

Add after line 306 (after the search_memory line):

```rust
prompt.push_str(
    "- When the user asks you to do multiple things in one message \
     (e.g. \"update both reminders\", \"create tasks for X and Y\"), \
     handle ALL of them in the same turn. Use multiple tool calls — \
     do not process one and ask about the rest. If you have enough \
     information for all actions, execute them all.\n",
);
```

### Change 2: Strengthen conversation continuity

Replace the existing line 306:

**Before:**
```rust
prompt.push_str(
    "- Use search_memory to find stored facts before asking the user to repeat information.\n",
);
```

**After:**
```rust
prompt.push_str(
    "- Before asking a clarifying question, check the conversation history — \
     the user may have already answered it in a previous message. \
     Never re-ask something the user already told you. \
     Also use search_memory to find stored facts before asking the user to repeat information.\n",
);
```

### File: `crates/mika-agent/src/prompt.rs`

Both changes are in `build_system_prompt()`, in the `## Tool Usage` section (lines 295-373).

## Acceptance Criteria

- [x] System prompt includes multi-action batching instruction in `## Tool Usage` section
- [x] System prompt includes strengthened conversation continuity instruction
- [x] Existing test `test_prompt_includes_soul_content` and other prompt tests still pass
- [x] New test verifying the multi-action instruction appears in the generated prompt
- [x] New test verifying the conversation continuity instruction appears in the generated prompt
- [x] `cargo test` passes (~925 tests)
- [x] `cargo clippy` clean

## Technical Considerations

- **Token budget:** Adds ~80 tokens to the system prompt. Negligible impact given the prompt is already ~2000+ tokens.
- **No architectural changes:** The tool execution loop (`process_tool_calls` in `agent.rs`) already processes multiple `ToolUse` blocks sequentially in a single turn. The max-steps limit (10) provides a natural guard.
- **Edge case — ambiguous multi-requests:** The instruction says "if you have enough information for all actions" — this gives the agent an out when the user's intent is genuinely ambiguous.
- **Edge case — over-batching:** The 10-step limit prevents runaway tool chains. The instruction is scoped to "multiple things in one message", not speculative batching.
- **Channel parity:** These are system prompt instructions, so they apply equally to CLI, Telegram, and server channels.

## Sources

- `crates/mika-agent/src/prompt.rs:295-373` — Tool Usage section of `build_system_prompt()`
- `crates/mika-agent/src/prompt.rs:306` — Current passive `search_memory` instruction
- `crates/mika-agent/src/agent.rs` — Agent loop with multi-tool-call support
