---
status: pending
priority: p2
issue_id: "221"
tags: [code-review, observability, skills-system]
dependencies: []
---

# No CLI Command to List or Inspect Skills

## Problem Statement
There is no `mika skills` or `mika skills list` command to view loaded skills, their tools, or their status. Users and developers have no way to verify which skills are active, what tools they provide, or debug skill loading issues without reading the source code.

## Findings
- No skill-related subcommand in `crates/mika-cli/src/commands/`
- The `SkillRegistry` has `skills()` accessor but it's not exposed to the CLI
- Agent-native principle: any action a user can take, an agent can also take (and vice versa)
- Missing observability for a core system

## Proposed Solutions

### Option 1: Add `mika skills` subcommand
- **Pros**: Full visibility, follows existing CLI patterns (mika status, mika memory, etc.)
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected Files**: `crates/mika-cli/src/commands/` (new file), `crates/mika-cli/src/main.rs`

## Acceptance Criteria
- [ ] `mika skills` lists all loaded skills with name, handler type, always_on status, tools
- [ ] Clear output format

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — agent-native-reviewer agent
