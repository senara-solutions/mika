---
status: pending
priority: p3
issue_id: 740
tags: [code-review, dashboard, quality]
dependencies: []
---

# Extract duplicated dashboard formatting helpers

## Problem Statement

`formatLatency`, `formatTokens`, and `sourceBadge`/`toolSourceBadge` are duplicated across 4-6 dashboard page files (LlmCallDetail, ToolCallDetail, TraceDetail, SessionDetail, LlmCalls, ToolCalls). This is pre-existing duplication that grows with each new page.

## Findings

- `formatLatency` is identical across 6 files
- `formatTokens` appears in 4 files with a minor inconsistency (hyphen vs em-dash for null — fixed in #361)
- `sourceBadge`/`toolSourceBadge` has two naming variants for the same logic
- `statusBadge`/`llmStatusBadge` is duplicated with minor text size differences

## Proposed Solutions

1. **Extract to `@senara-solutions/ui`** — alongside existing `formatTimestamp` and `formatRelativeTime`. Pros: single source of truth. Cons: requires UI package rebuild.
2. **Extract to `dashboard/src/utils/formatters.ts`** — local shared module. Pros: no package rebuild needed. Cons: not available to other packages.

## Recommended Action

Option 2 (local utils) for speed, with Option 1 as follow-up.

## Technical Details

- **Affected files**: LlmCallDetail.tsx, ToolCallDetail.tsx, TraceDetail.tsx, SessionDetail.tsx, LlmCalls.tsx, ToolCalls.tsx
- ~30 LOC consolidated

## Acceptance Criteria

- [ ] `formatLatency`, `formatTokens` extracted to shared location
- [ ] `sourceBadge` unified naming and extracted
- [ ] All pages import from shared location
