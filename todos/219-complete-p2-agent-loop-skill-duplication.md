---
status: complete
priority: p2
issue_id: "219"
tags: [code-review, architecture, skills-system]
dependencies: []
---

# Duplicated Skill Integration in run_agent_inner and run_silent_inner

## Problem Statement
The skill matching, prompt snippet loading, and tool resolution logic is duplicated nearly identically between `run_agent_inner` and `run_silent_inner` (~15 lines each). This violates DRY and means changes must be made in two places.

## Findings
- Location: `crates/mika-agent/src/agent.rs` — both `run_agent_inner` and `run_silent_inner`
- Same 3-way branch (matched / no-skills / no-match)
- Same resolve_matched_skills call pattern
- Same skill_tool_map threading
- Pre-existing issue (#078, #114) — skills integration adds more duplication

## Proposed Solutions

### Option 1: Extract shared skill resolution into a helper function
- **Pros**: Single source of truth, easy to maintain
- **Cons**: Need to design clean interface
- **Effort**: Small
- **Risk**: Low

## Technical Details
- **Affected Files**: `crates/mika-agent/src/agent.rs`

## Acceptance Criteria
- [ ] Skill resolution logic extracted to single function
- [ ] Both agent loops call the shared helper

## Work Log
### 2026-02-25 - Created from code review
**By:** Claude Code Review — pattern-recognition-specialist + architecture-strategist agents
