---
status: complete
priority: p2
issue_id: "558"
tags: [code-review, naming, quality]
dependencies: []
---

# get_pending_reminder_tasks Name Mismatches Broadened Scope

## Problem Statement

`get_pending_reminder_tasks` was broadened from filtering `trigger_type IN ('time', 'recurring') AND action_type = 'send_message'` to `action_type != 'run_skill'`. It now returns internal engine tasks (`resume_agent`, `invoke_orchestrator`) that are not user-facing reminders. The method name is misleading and the footer badge may show inflated counts.

## Findings

- **Found by:** Architecture Strategist, Security Sentinel, Pattern Recognition, Agent-Native Reviewer (4/8 agents)

## Proposed Solutions

1. Rename to `get_user_visible_tasks` or `get_active_tasks`
2. Tighten the filter to only include user-meaningful tasks:
   ```sql
   WHERE (action_type = 'send_message'
     OR (trigger_type = 'callback' AND action_type = 'resume_agent'))
   ```

**Effort:** Small

## Acceptance Criteria

- [ ] Method name reflects its actual scope
- [ ] Footer badge shows only user-meaningful task counts

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | 4/8 agents flagged naming issue |
| 2026-03-07 | Approved during triage | Rename + tighten filter |
