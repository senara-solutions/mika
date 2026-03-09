---
status: complete
priority: p2
issue_id: "584"
tags: [code-review, architecture]
dependencies: []
---

# Agent-Native Parity Gap: 0/9 Dashboard Capabilities Have Agent Tools

## Problem Statement
The dashboard exposes 9 read-only data views that operators can see, but agents have no tools to query the same data. Agents cannot inspect the unified timeline, traces, past session history, or audit events. This creates a significant visibility gap.

## Findings
- **Source:** Agent-Native Reviewer
- Key missing tools: `query_timeline` (wrapping unified_timeline VIEW), `get_session_messages` (past conversation replay), `list_audit_events` (self-introspection)
- `list_agents` tool returns less data than dashboard agents view (no stats)

## Proposed Solutions
Add 2-3 read-only agent tools wrapping existing DB methods:
1. `query_timeline` — wraps `unified_timeline` VIEW, scoped to own agent_id for non-orchestrators
2. `get_session_messages` — browse past session conversations
3. `list_audit_events` — review own memory mutations

## Acceptance Criteria
- [ ] Agents can query their own timeline events
- [ ] Agents can browse past session history
- [ ] Orchestrator agents can query cross-agent data

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Agent-Native Reviewer: 0/9 full parity |

## Resources
- PR #89
