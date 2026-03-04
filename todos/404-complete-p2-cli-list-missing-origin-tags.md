---
status: complete
priority: p2
issue_id: "404"
tags: [code-review, agent-native, marketplace, pr-56]
dependencies: []
---

# CLI list_skills doesn't show origin tags

## Problem Statement

The agent's `list_skills` tool shows `[built-in]`, `[marketplace]`, `[custom]` origin tags, but the CLI `mika skills list` does not display origin tags at all. Users can't see at a glance which skills are marketplace-installed vs custom.

## Findings

- **Source**: agent-native-reviewer
- **File**: `crates/mika-cli/src/commands/skills.rs:57-82`

## Proposed Solutions

### Option A: Add origin tags to CLI output (Recommended)

Read the lock file once and display origin tags matching the agent tool format.

- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] `mika skills list` shows `[built-in]`, `[marketplace]`, `[custom]` tags
- [ ] Consistent with agent `list_skills` tool output

## Resources

- `crates/mika-cli/src/commands/skills.rs:57-82`
- `crates/mika-agent/src/tools/list_skills.rs:54-60` (reference)
