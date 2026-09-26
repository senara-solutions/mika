---
status: complete
priority: p2
issue_id: "048"
tags: [code-review, agent-native, tools, rust-v2]
dependencies: ["046"]
---

# No Agent Equivalent for /reset CLI Command

## Problem Statement
The CLI `/reset <block>` command resets a core memory block to its default value, but the agent has no equivalent capability. If the user asks "reset my user summary", the agent cannot comply because it doesn't know the default values and has no reset action.

**Location:** `crates/mika-agent/src/cli.rs:164-200` (CLI-only), no tool equivalent

**Reported by:** agent-native-reviewer

## Proposed Solutions

### Option A: Add "reset" action to update_core_memory (Recommended)
Add a `reset` action alongside replace/append/remove_line. The tool looks up defaults from the shared constant (depends on #046).
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria
- [ ] Agent can use update_core_memory with action "reset" to restore defaults
- [ ] Behavior matches CLI /reset command

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | |
