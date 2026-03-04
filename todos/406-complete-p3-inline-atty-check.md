---
status: complete
priority: p3
issue_id: "406"
tags: [code-review, simplicity, marketplace, pr-56]
dependencies: []
---

# Inline atty_check() helper

## Problem Statement

`atty_check()` is a 3-line function wrapping `std::io::stdin().is_terminal()`, called exactly once. Inline it.

## Findings

- **Source**: code-simplicity-reviewer
- **File**: `crates/mika-cli/src/commands/skills.rs:431-433`

## Resources

- `crates/mika-cli/src/commands/skills.rs:431-433`
