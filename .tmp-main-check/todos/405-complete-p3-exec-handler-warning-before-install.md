---
status: complete
priority: p3
issue_id: "405"
tags: [code-review, security, marketplace, pr-56]
dependencies: []
---

# Exec handler warning shown after install, no confirmation

## Problem Statement

The install flow warns about exec handlers **after** the skill is already installed. The warning should appear before installation with a confirmation prompt.

## Findings

- **Source**: security-sentinel
- **File**: `crates/mika-cli/src/commands/skills.rs:390-393`

## Proposed Solutions

Move exec handler detection and warning before the copy step, require y/N confirmation via `dialoguer::Confirm`.

## Resources

- `crates/mika-cli/src/commands/skills.rs:390-393`
