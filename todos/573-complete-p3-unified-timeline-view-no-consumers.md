---
status: complete
priority: p3
issue_id: "573"
tags: [code-review, yagni, simplicity, observability]
dependencies: []
---

# unified_timeline VIEW has no production consumers (YAGNI consideration)

## Problem Statement

The `unified_timeline` VIEW is defined in both `migrate_v1` and `migrate_v4_to_v5` (~65 lines duplicated), but no production code queries it. The only consumers are two test functions. The VIEW SQL is also duplicated across two migration paths.

## Findings

- **Source:** Code Simplicity Reviewer
- **Counterpoint:** Architecture Strategist and Agent-Native Reviewer both noted the VIEW is a good primitive for future agent introspection tooling
- **Impact:** ~65 lines of duplicated SQL, speculative infrastructure

## Proposed Solutions

### Option A: Keep but extract to constant (Recommended)
Extract the VIEW SQL into a `const` to prevent the two copies from drifting apart. The VIEW is zero-cost (no storage overhead) and provides immediate value for debugging via `sqlite3` CLI.

- **Effort:** Small (10 min)
- **Risk:** None

### Option B: Remove entirely (YAGNI purist)
Delete the VIEW and its tests. Add it back when a real consumer exists.

- **Effort:** Small (10 min)
- **Risk:** Low — but VIEW is useful for manual debugging even without code consumers

## Acceptance Criteria

- [ ] VIEW SQL is not duplicated (either extracted to const or removed)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from PR #88 code review | Zero-cost VIEW is useful for debugging even without code consumers |
