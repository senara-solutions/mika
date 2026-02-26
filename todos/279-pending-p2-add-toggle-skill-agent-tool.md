---
status: pending
priority: p2
issue_id: 279
tags: [code-review, agent-native, skills]
dependencies: []
---

# Add toggle_skill Agent Tool

## Problem Statement

The CLI can enable/disable skills via `mika skills enable/disable <name>`, but the agent has no equivalent. If a user says "disable the web-search skill" via Telegram, the agent cannot comply. The agent can create skills but cannot manage their lifecycle after creation.

## Findings

- **Agent-native reviewer**: "The CLI can enable and disable skills but the agent has no equivalent"
- Enable/disable is implemented via `.disabled` marker file in `crates/mika-cli/src/commands/skills.rs:271-294`
- Simple filesystem primitive (create/remove a file)

## Proposed Solutions

### Option A: Add `toggle_skill` tool (Recommended)
- Create `crates/mika-agent/src/tools/toggle_skill.rs`
- Accept skill name and enable/disable action
- Create or remove the `.disabled` marker file
- Pros: Simple, follows existing filesystem patterns
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] Agent can enable and disable skills by name
- [ ] Tool validates skill name and verifies skill exists
- [ ] Registered in `default_tools()`

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-26 | Created from code review | Agent-native parity gap identified |
