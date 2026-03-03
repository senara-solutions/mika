---
status: complete
priority: p3
issue_id: "408"
tags: [code-review, architecture, marketplace, pr-56]
dependencies: []
---

# Revert unnecessary pub(crate) visibility on unused validators

## Problem Statement

`validate_description`, `validate_system_prompt`, and `validate_keywords` were widened from `pub(super)` to `pub(crate)` but are not used by the marketplace code. Only `validate_skill_name` and `verify_skill_path` need the visibility bump.

## Findings

- **Source**: code-simplicity-reviewer, architecture-strategist
- **File**: `crates/mika-agent/src/tools/create_skill.rs`

## Resources

- `crates/mika-agent/src/tools/create_skill.rs:36,50,64`
