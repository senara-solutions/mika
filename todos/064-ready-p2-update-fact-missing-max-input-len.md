---
status: ready
priority: p2
issue_id: "064"
tags: [code-review, security, quality, rust-v2]
dependencies: []
---

# update_fact Tool Missing MAX_INPUT_LEN Validation

## Problem Statement

Every other tool (`update_core_memory`, `store_fact`, `search_memory`) imports and validates against `MAX_INPUT_LEN` (10,000 chars). `update_fact` does not import or use it. While current inputs are enum-constrained ("completed", "cancelled"), this breaks the defense-in-depth pattern and will be a gap when the tool is extended to support other categories.

**Why it matters:** Consistency in input validation patterns prevents future security gaps when the tool is extended.

## Findings

- **Source:** security-sentinel, pattern-recognition-specialist
- **Location:** `crates/mika-agent/src/tools/update_fact.rs:6` (missing import), lines 52-71 (no length check)
- **Evidence:** `super::MAX_INPUT_LEN` not imported; all 3 other tools import and validate it

## Proposed Solutions

### Option A: Add MAX_INPUT_LEN import and validation (Recommended)
- Import `MAX_INPUT_LEN` from `super`
- Add length validation on string inputs (category, status)
- **Pros:** Consistent with all other tools, future-proof
- **Cons:** ~5 lines added
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] `update_fact` imports `MAX_INPUT_LEN`
- [ ] String inputs validated against length limit
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review of commit 3619d13 | All 4 tools should share the same validation pattern |
