---
status: complete
priority: p2
issue_id: "581"
tags: [code-review, performance]
dependencies: []
---

# Two Sequential DB Round-Trips Per Paginated Endpoint

## Problem Statement
Every paginated handler makes two sequential `await` calls through AsyncDatabase (data + count). This means two channel round-trips and a TOCTOU race where data can change between calls.

## Findings
- **Source:** Performance Oracle, Code Simplicity Reviewer
- **Location:** All paginated handlers in `dashboard.rs` (5 occurrences)

## Proposed Solutions
Combine data + count into single `with_db` calls returning `(Vec<T>, u64)`.

## Acceptance Criteria
- [ ] Each paginated endpoint makes one DB dispatch, not two
- [ ] Response still contains accurate total count

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Performance Oracle + Simplicity Reviewer flagged |

## Resources
- PR #89
