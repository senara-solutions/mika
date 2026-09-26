---
status: pending
priority: p2
issue_id: "579"
tags: [code-review, performance]
dependencies: []
---

# handle_agent_detail Loads All Agents to Find One

## Problem Statement
`handle_agent_detail` calls `list_agents_with_stats()` which fetches ALL agents with correlated subqueries, then searches in Rust with `.find()`. This is O(N) correlated subqueries when only one agent is needed. The agent's basic info already exists in `resolve_agent()`.

## Findings
- **Source:** Performance Oracle, Architecture Strategist, Code Simplicity Reviewer
- **Location:** `crates/mika-agent/src/server/dashboard.rs` lines 230-247

## Proposed Solutions
Add `get_agent_with_stats(agent_id)` to `Database` that does a single-row query.

## Acceptance Criteria
- [ ] `handle_agent_detail` does not call `list_agents_with_stats`
- [ ] Single-row query for the requested agent

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | All 3 reviewers flagged this |

## Resources
- PR #89
