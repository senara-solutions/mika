---
status: complete
priority: p1
issue_id: "412"
tags: [code-review, agent-native, reflection]
dependencies: []
---

# Reflection Prompt Instructs "Remove Stale Information" But No Delete Tool Exists

## Problem Statement

The reflection prompt in `agent.rs:1145` instructs:
> "Scan for duplicate or redundant facts. Consolidate them. Remove stale information that's no longer relevant."

There is no `delete_fact`, `remove_fact`, or `archive_fact` tool anywhere in the codebase. `update_fact` only supports updating commitment status (`completed`/`cancelled`) — it cannot delete people, preferences, or events.

The agent will be instructed to do something it physically cannot do. It may hallucinate tool calls, produce confusing error messages, or waste tool steps trying to find a way to delete facts.

## Findings

- **Agent-native reviewer**: "This is an Orphan Feature in reverse: the prompt describes a capability that has no tool backing"
- No `delete_fact` tool exists in `crates/mika-agent/src/tools/`
- `update_fact` only supports commitment status updates

## Proposed Solutions

### Option A: Rewrite prompt to match available capabilities (Recommended)
Replace "Remove stale information" with language that matches what the agent CAN do:
- "Mark stale commitments as cancelled using update_fact"
- "Consolidate duplicate facts by updating existing ones with more complete information"
- "Update outdated facts with current information using store_fact"
- **Pros**: Quick fix, no new tools needed, honest about capabilities
- **Cons**: Agent still can't truly delete stale facts
- **Effort**: Small
- **Risk**: Low

### Option B: Add a delete_fact tool
Create a new tool that can archive/remove facts by ID and category.
- **Pros**: Full capability match with prompt intent
- **Cons**: More code, needs careful design (soft delete vs hard delete), scope creep for this PR
- **Effort**: Medium
- **Risk**: Medium (feature creep)

## Recommended Action

Option A for this PR. Option B can be a follow-up if needed.

## Technical Details

- **Affected file**: `crates/mika-agent/src/agent.rs` (lines 1140-1157, reflection trigger context string)

## Acceptance Criteria

- [ ] Reflection prompt only references tools that actually exist
- [ ] Prompt lists specific tool names the agent should use (update_core_memory, store_fact, update_fact, search_memory)
- [ ] No instruction to "remove" or "delete" facts unless a corresponding tool exists

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-03 | Identified during code review | Prompt-tool capability mismatch |

## Resources

- PR #59: periodic memory reflection
