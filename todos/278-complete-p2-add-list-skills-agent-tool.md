---
status: pending
priority: p2
issue_id: 278
tags: [code-review, agent-native, skills]
dependencies: []
---

# Add list_skills Agent Tool

## Problem Statement

The agent can create skills via `create_skill` but cannot discover what skills already exist. This is a Context Starvation anti-pattern: the agent can write to the skills subsystem but cannot read from it. If a user asks "what skills do I have?", the agent cannot answer.

## Findings

- **Agent-native reviewer**: "The agent has `create_skill` but cannot discover what skills already exist"
- **Pattern**: CLI has `mika skills list` and TUI has `/skills`, but no agent tool equivalent
- The `scan_skills_dir()` function already exists in `crates/mika-agent/src/skills/index.rs` and can be reused

## Proposed Solutions

### Option A: Add `list_skills` tool (Recommended)
- Create `crates/mika-agent/src/tools/list_skills.rs`
- Read from `home_dir/skills/` using `scan_skills_dir()`
- Return name, description, enabled status, tool count, keywords
- Pros: Read primitive with no business logic, follows existing patterns
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] Agent can list all installed skills with name, description, and status
- [ ] Tool follows existing tool patterns (TestHarness, ToolOutput, etc.)
- [ ] Registered in `default_tools()`

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-26 | Created from code review | Agent-native parity gap identified |
