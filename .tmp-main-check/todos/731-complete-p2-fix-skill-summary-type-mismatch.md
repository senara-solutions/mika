---
status: pending
priority: p2
issue_id: "731"
tags: [code-review, quality, dashboard, observability]
dependencies: []
---

# Fix SkillSummary TypeScript type mismatch

## Problem Statement

The `SkillSummary` interface in `toolCalls.ts` expects `{name, source, handler_type}` but the API returns `{"loaded_skills": ["skill1", "skill2"], "skill_count": 2}` — just string names. The Skills tab renders blank `source` and `handler_type` columns.

## Findings

- **Agents**: code-simplicity-reviewer, architecture-strategist
- **File**: `dashboard/src/api/toolCalls.ts` (SkillSummary interface), `dashboard/src/pages/SessionDetail.tsx` (Skills tab)

## Proposed Solutions

Simplify the Skills tab to render skill names as a simple list (matching what the API provides). Remove unused `source`/`handler_type` columns.

## Acceptance Criteria

- [ ] Skills tab renders loaded skill names correctly
- [ ] No blank columns
