---
status: pending
priority: p2
issue_id: "067"
tags: [code-review, agent-native, rust-v2]
dependencies: []
---

# System Prompt Does Not Mention update_fact or Reset Capabilities

## Problem Statement

The Instructions section of the system prompt tells the agent to "Track people, commitments, preferences, and events" but does not mention that it can mark commitments as completed/cancelled via `update_fact`, nor that it can `reset` a core memory block to defaults. While tool definitions are sent alongside the prompt, explicit instructions significantly improve LLM tool discovery and correct usage.

**Why it matters:** The agent may not discover it can complete commitments or reset memory without explicit prompting.

## Findings

- **Source:** agent-native-reviewer
- **Location:** `crates/mika-agent/src/prompt.rs:88-101`
- **Evidence:** Instructions mention "Track..." but not "Update..." or "Reset..."

## Proposed Solutions

### Option A: Add capability bullets to Instructions (Recommended)
- Add: "Mark commitments as completed or cancelled using the update_fact tool."
- Add mention of reset action in core memory instructions
- **Pros:** Improves tool discovery, matches CLI capabilities
- **Cons:** ~2 lines added to prompt
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] System prompt mentions update_fact capability
- [ ] System prompt mentions reset action for core memory
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review of commit 3619d13 | Tool definitions alone aren't sufficient for LLM discovery |
