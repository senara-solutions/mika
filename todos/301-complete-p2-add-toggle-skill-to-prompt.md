---
status: complete
priority: p2
issue_id: 301
tags: [code-review, agent-native, prompt]
dependencies: []
---

# Add toggle_skill mention to Tool Usage prompt

## Problem Statement

The Tool Usage section in the system prompt mentions `create_skill` and `list_skills` but not `toggle_skill`. The agent may not realize it can enable/disable skills.

## Findings

- **Agent-Native Reviewer:** The prompt says "create new skills using create_skill" and "built-in skills (use list_skills to see which)" but does not mention toggle_skill. The agent may not know it can enable/disable skills unless it discovers the tool through its tool definitions.

## Proposed Solutions

### Solution A: Add one line to Tool Usage section

In `prompt.rs`, after the "built-in skills" line, add:
```rust
prompt.push_str(
    "- You can enable or disable skills with toggle_skill.\n",
);
```

- Effort: Small
- Risk: Low

## Technical Details

- File: `crates/mika-agent/src/prompt.rs`
