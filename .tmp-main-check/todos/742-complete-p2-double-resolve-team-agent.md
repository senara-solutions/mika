---
status: pending
priority: p2
issue_id: "742"
tags: [code-review, simplicity, bug-adjacent]
dependencies: []
---

# Remove duplicate resolve_github_token in run_team_agent_inner_impl

## Problem Statement

In `agent.rs` `run_team_agent_inner_impl`, `resolve_github_token` is called twice from the same params — once as `team_resolved_github_token` for context injection and once as `resolved_github_token` for ToolContext. Both resolve the same token, wasting an async call.

## Proposed Solutions

### Option A: Reuse first resolution (Recommended)
- Remove the second `resolve_github_token` call, use `team_resolved_github_token` everywhere
- **Effort:** Small (6 LOC)
- **Risk:** Low

## Acceptance Criteria

- [ ] Only one `resolve_github_token` call in `run_team_agent_inner_impl`
