---
status: pending
priority: p2
issue_id: "105"
tags: [code-review, agent-native, prompt]
dependencies: []
---

# Add search_memory to conversation prompt instructions

## Problem Statement
The conversation prompt's Instructions section does not mention `search_memory`. The agent may not know it can search across all memory categories. The silent prompt was updated (todo #093) but the conversation prompt was missed.

## Findings
- File: `crates/mika-agent/src/prompt.rs` (build_conversation_prompt function)
- Conversation prompt mentions store_fact, update_core_memory, create_reminder, etc.
- `search_memory` is not mentioned in the instructions
- Agent relies on tool definitions alone for discovery — explicit prompt guidance increases reliability
- Flagged by: Agent-Native Reviewer (Warning)

## Proposed Solutions

### Option 1: Add search_memory to Instructions section (Recommended)
Add a line like: "Use `search_memory` to find information across all memory categories before asking the user."
**Effort:** Trivial
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/prompt.rs`

## Acceptance Criteria
- [ ] Conversation prompt mentions search_memory
- [ ] Tests updated if prompt assertions exist
- [ ] Agent has guidance on when to use search_memory

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Agent-Native Reviewer found search_memory missing from conversation prompt
