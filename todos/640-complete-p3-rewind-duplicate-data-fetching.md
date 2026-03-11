---
status: complete
priority: p3
issue_id: "640"
tags: [code-review, simplicity, rewind]
dependencies: []
---

# Duplicate data fetching in execute_rewind (preview re-fetches)

## Problem Statement

`execute_rewind()` calls `build_reversal_previews()` which fetches audit events and builds previews, but the execute path then separately processes audit events. The preview data could be reused instead of re-querying.

## Findings

- **Source:** Simplicity review agent
- **Location:** `crates/mika-agent/src/rewind.rs` — `execute_rewind()`
- The preview step queries audit events; the execute step re-processes them
- This is a minor efficiency concern, not a correctness issue

## Proposed Solutions

### Option A: Refactor to reuse preview data in execution
Pass the already-built previews into the execution logic.
- **Effort:** Medium — requires restructuring the execute flow
- **Risk:** Low

### Option B: Accept the duplication
The data is small and SQLite queries are fast. Not worth the refactor complexity.
- **Effort:** None
- **Risk:** None

## Acceptance Criteria

- [ ] Either: Data fetching consolidated, OR: accepted as-is given minimal performance impact
