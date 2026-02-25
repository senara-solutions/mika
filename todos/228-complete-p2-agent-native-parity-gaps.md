---
status: complete
priority: p2
issue_id: 228
tags: [code-review, agent-native, slash-commands]
dependencies: []
---

# Agent-Native Parity Gaps for Slash Command Features

## Problem Statement

The slash commands expose status, skills, and soul information to the user via CLI commands, but the agent has no equivalent tools to access this information. This breaks agent-native parity — any action a user can take should also be available to the agent.

**Why it matters:** The agent can't proactively check system health, discover available skills, or read the soul configuration, limiting its ability to help users autonomously.

## Findings

**Source:** Agent-Native Reviewer (2.5/5 compliance score)

**Missing agent capabilities:**
1. Agent cannot check system health (`/status` equivalent)
2. Agent cannot discover loaded skills (`/skills` equivalent)
3. Agent cannot read soul.md (`/soul` equivalent)
4. No JSON output option for programmatic access

## Proposed Solutions

### Solution A: Defer to future sprint (Recommended)
- These are new feature requests, not bugs in the current slash-commands PR
- The agent already has memory, reminder, and messaging tools
- Status/skills/soul tools can be added as new skills in the skills system
- **Pros:** Keeps current PR focused, uses existing extensibility mechanism
- **Cons:** Delayed parity
- **Effort:** N/A (deferred)
- **Risk:** None

### Solution B: Add agent tools now
- Create `check_status`, `list_skills`, `read_soul` tools
- Add to ToolRegistry or as builtin skills
- **Pros:** Immediate parity
- **Cons:** Scope creep, unrelated to slash-commands PR
- **Effort:** Medium
- **Risk:** Low

## Recommended Action

Solution A — defer to a separate PR/sprint. The slash-commands PR is client-side only by design.

## Technical Details

- **Affected files:** Would be `crates/mika-agent/src/tools/` (new tools)

## Acceptance Criteria

- [ ] Decision documented (defer vs implement now)
- [ ] If implementing: new tools added to ToolRegistry with tests

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from code review | Agent-native reviewer flagged parity gaps |

## Resources

- PR branch: `feat/slash-commands`
