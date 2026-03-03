---
status: complete
priority: p2
issue_id: "403"
tags: [code-review, agent-native, marketplace, pr-56]
dependencies: []
---

# delete_skill tool description doesn't mention marketplace skills

## Problem Statement

The `delete_skill` tool description says "Permanently delete a custom skill" but marketplace skills can also be deleted (and the lock entry is cleaned up). The agent might hesitate to delete a `[marketplace]` skill because the description says "custom skill."

## Findings

- **Source**: agent-native-reviewer
- **File**: `crates/mika-agent/src/tools/delete_skill.rs:23-26`

## Proposed Solutions

### Option A: Update description (Recommended)

Change "custom skill" to "custom or marketplace skill" in the tool description.

- Effort: Trivial (one word)
- Risk: None

## Acceptance Criteria

- [ ] Tool description mentions marketplace skills
- [ ] Tests still pass

## Resources

- `crates/mika-agent/src/tools/delete_skill.rs:23-26`
