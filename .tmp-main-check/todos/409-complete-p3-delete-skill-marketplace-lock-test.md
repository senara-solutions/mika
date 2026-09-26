---
status: complete
priority: p3
issue_id: "409"
tags: [code-review, quality, marketplace, pr-56]
dependencies: []
---

# Missing test for marketplace lock cleanup in delete_skill

## Problem Statement

The `delete_skill` tests don't verify that marketplace lock entries are removed when deleting a marketplace skill. The lock cleanup code (lines 74-79) is only tested indirectly via the install module.

## Findings

- **Source**: agent-native-reviewer
- **File**: `crates/mika-agent/src/tools/delete_skill.rs:90-278`

## Proposed Solutions

Add a test that creates a skill directory, writes a marketplace lock entry, deletes via `DeleteSkillTool`, and verifies the lock entry is removed.

## Resources

- `crates/mika-agent/src/tools/delete_skill.rs:74-79`
