---
status: complete
priority: p3
issue_id: 425
tags: [code-review, feature, agent-native]
dependencies: []
---

# Add read_file and list_files builtin tools for home directory

## Problem Statement

`write_agent_file` is a builtin tool (always available, including silent mode), but `read_file` is a skill-based exec handler (filtered out in heartbeat/reflection mode, requires jq). This creates an asymmetry — an agent could write a file but be unable to read it back in certain contexts. There is no `list_files` for the home directory at all.

## Findings

- Source: agent-native-reviewer
- `write_agent_file` is available in silent mode; `read_file` skill is not (exec handlers filtered by `safe_always_on_skills()`)

## Proposed Solutions

### Option A: Add `read_file` and `list_files` builtins
- **Pros:** Full parity, symmetric file operations
- **Effort:** Medium (follow write_agent_file pattern)

## Acceptance Criteria

- [ ] `read_file` builtin with same security model as `write_agent_file`
- [ ] `list_files` builtin for home directory
- [ ] Both registered in `default_tools()`
- [ ] Prompt documentation added
