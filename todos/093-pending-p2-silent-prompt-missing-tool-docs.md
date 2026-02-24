---
status: pending
priority: p2
issue_id: "093"
tags: [code-review, agent-native]
dependencies: []
---

# Add tool guidance to silent-mode system prompt

## Problem Statement
The silent-mode prompt (`build_silent_prompt`) mentions only `send_message` but does not document other available tools (search_memory, store_fact, update_core_memory, create_reminder). The conversation prompt has rich tool instructions, but the silent prompt has none. While tool definitions are sent via the API, explicit prompt guidance increases reliability.

## Findings
- File: `crates/mika-agent/src/prompt.rs`, `build_silent_prompt` function
- Conversation prompt (lines 104-124) has detailed Instructions section with tool guidance
- Silent prompt mentions `send_message` in the trigger context but nothing else
- Agent has full tool access in silent mode (shared `process_tool_calls`), but no guidance on when to use memory tools during heartbeat/reminders
- Flagged by: Agent-Native Reviewer (Warning)

## Proposed Solutions

### Option 1: Add brief tool capability summary (Recommended)
Add after the Silent Mode section:
```
## Available Tools
You have access to all tools. Use them as appropriate:
- search_memory / store_fact / update_core_memory: Read and update the user's memory
- create_reminder / list_reminders / cancel_reminder: Manage reminders
- send_message: Contact the user (required in silent mode for output)
```
**Pros:** Guides tool usage without duplicating full instructions
**Effort:** Small
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/prompt.rs`

## Acceptance Criteria
- [ ] Silent prompt includes brief tool capability summary
- [ ] All 8 tools mentioned
- [ ] Tests updated

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified context parity gap between conversation and silent prompts
