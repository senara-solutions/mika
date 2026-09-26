---
status: complete
priority: p3
issue_id: 277
tags: [code-review, architecture, teams]
dependencies: []
---

# Consider follow-up injection for team agent empty responses

## Problem Statement

`run_team_agent_inner` returns `Ok(None)` for empty text turns without the follow-up re-prompt that `run_agent_inner` uses. If a team specialist does all its work through tool calls (e.g., writing workspace files) but produces no text, the team engine gets an empty string via `unwrap_or_default()`, which could cause `parse_task_assignments` to fail.

## Findings

- **File**: `crates/mika-agent/src/agent.rs:619-627`
- **Impact**: Low-Medium — team agent results feed into orchestrator, not directly to users
- **Found by**: agent-native-reviewer, architecture-strategist

## Proposed Solution

Either:
1. Add the same follow-up injection to `run_team_agent_inner`
2. Document explicitly why team agents skip it (e.g., "team agents write to files, so empty text is acceptable")

## Acceptance Criteria

- [ ] Team agent empty response behavior is either fixed or documented
- [ ] If fixed, tests cover the follow-up injection path
