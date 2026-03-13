---
status: complete
priority: p3
issue_id: "491"
tags: [code-review, agent-native, quality]
dependencies: []
---

# Silent-Mode Prompt Does Not Document read_file or list_files Builtins

## Problem Statement

`build_silent_prompt` in `prompt.rs` does not include the Tool Usage section that documents
`read_file`, `list_files`, or `write_agent_file`. Both `read_file` and `list_files` are registered
in `default_tools()` and ARE available in heartbeat/reflection silent runs (builtin-handler
tools pass `safe_always_on_skills` filtering). However, the agent in silent mode has no prompt
documentation telling it these tools exist. A heartbeat agent that wants to read its own notes
file to formulate a proactive message cannot discover this capability from the prompt alone.

## Findings

- **Source**: agent-native-reviewer review
- **Location**: `crates/mika-agent/src/prompt.rs` (build_silent_prompt function)
- Normal prompt at `prompt.rs:312–342` documents write_agent_file, read_file, list_files with home dir path
- Silent prompt has no equivalent Tool Usage block

## Proposed Solutions

### Option A: Add minimal Tool Usage block to build_silent_prompt (Recommended)
Add a brief section listing `read_file`, `list_files`, `write_agent_file` with the home dir path,
similar to the normal prompt at lines 312–342. Reuse the same helper function that builds the
file-tools section.
- **Effort**: Small | **Risk**: None

### Option B: Include full tool documentation in silent prompt
Add the complete Tool Usage section (all tools) to the silent prompt.
- **Cons**: Increases prompt size, may push silent prompt over token budget
- **Effort**: Tiny | **Risk**: Low

## Acceptance Criteria

- [ ] `build_silent_prompt` documents `read_file` and `list_files` with their home dir base path
- [ ] Heartbeat agent can discover file tools from prompt alone
- [ ] Silent prompt token budget is not significantly increased

## Work Log

- 2026-03-06: Identified by agent-native-reviewer of feat/unified-task-engine
