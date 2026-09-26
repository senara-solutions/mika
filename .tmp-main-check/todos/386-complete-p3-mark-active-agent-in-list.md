---
status: complete
priority: p3
issue_id: 386
tags: [code-review, agent-native, ux]
dependencies: []
---

# Mark the active agent in list_agents output

## Problem Statement

The CLI `agents list` shows `(active)` markers, but the `list_agents` tool does not indicate which agent is currently calling. An LLM might try to delegate to itself.

## Findings

- **Source:** Agent-Native Reviewer
- The tool struct has no access to the current agent's name
- The system prompt partially addresses this by listing agents but does not say "you are agent X"

## Proposed Solutions

### Option 1: Add current agent name to system prompt (Recommended)
Add "You are the '{name}' agent" in the Agents & Teams section. No tool changes needed.
- **Effort:** Small
- **Risk:** None

### Option 2: Add agent name field to ListAgentsTool struct
Pass the calling agent's name and mark it in output.
- **Effort:** Medium
- **Risk:** Requires threading the name through tool construction
