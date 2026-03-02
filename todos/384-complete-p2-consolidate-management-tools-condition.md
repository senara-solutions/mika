---
status: complete
priority: p2
issue_id: 384
tags: [code-review, architecture, dry]
dependencies: []
---

# Consolidate management tools registration condition

## Problem Statement

The condition `agents.len() > 1 || !teams.is_empty()` is duplicated in three places: `chat.rs:54`, `ask.rs:20`, and `server/mod.rs:144`. This is a DRY violation where a policy decision is scattered across files.

## Findings

- **Source:** Architecture Strategist
- The prompt builder (`prompt.rs:192-228`) also independently checks the same condition

## Proposed Solutions

### Option 1: Wrapper function that internalizes the condition (Recommended)
Create `management_tools_if_needed(home_dir, settings) -> Vec<Box<dyn Tool>>` that calls `list_agents`/`list_teams` internally and returns empty vec when condition is not met.
- **Effort:** Small
- **Risk:** None

### Option 2: Keep as-is
The duplication is only 3 lines per site and the condition is simple.
- **Effort:** None
- **Risk:** Drift if condition changes
