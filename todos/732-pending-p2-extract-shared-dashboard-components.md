---
status: pending
priority: p2
issue_id: "732"
tags: [code-review, quality, dashboard, observability]
dependencies: []
---

# Extract shared dashboard formatting helpers and table components

## Problem Statement

`formatTokens`, `formatLatency`, `statusBadge`, `toolSourceBadge` are copy-pasted across 3-4 dashboard files. LLM calls table and tool calls table markup duplicated in 2-3 places each. ~320 LOC of duplication.

## Findings

- **Agent**: code-simplicity-reviewer
- **Files**: `LlmCalls.tsx`, `ToolCalls.tsx`, `SessionDetail.tsx`, `TraceDetail.tsx`

## Proposed Solutions

1. Extract helpers to `dashboard/src/utils/observability.tsx`
2. Extract `<LlmCallsTable>` and `<ToolCallsTable>` reusable components

## Acceptance Criteria

- [ ] Shared helpers in one file
- [ ] Table components reused across pages
- [ ] ~320 LOC reduction
