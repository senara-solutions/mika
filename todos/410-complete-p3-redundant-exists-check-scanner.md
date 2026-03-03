---
status: complete
priority: p3
issue_id: "410"
tags: [code-review, simplicity, marketplace, pr-56]
dependencies: []
---

# Redundant exists() check in scan_repo_for_skills

## Problem Statement

In `try_load_candidate`, `skill_dir.join("tools.json").exists()` is called before `read_to_string` which already returns `Err` for missing files. The `exists()` check is an extra stat syscall that provides no value.

## Findings

- **Source**: performance-oracle
- **File**: `crates/mika-agent/src/skills/marketplace.rs:182-191`

## Proposed Solutions

Remove the `exists()` check — `read_to_string().ok()` already handles the missing case.

## Resources

- `crates/mika-agent/src/skills/marketplace.rs:182-191`
