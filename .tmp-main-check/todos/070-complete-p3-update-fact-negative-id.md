---
status: complete
priority: p3
issue_id: "070"
tags: [code-review, security, rust-v2]
dependencies: []
---

# update_fact Accepts Negative IDs

## Problem Statement

`update_fact` validates `id == 0` but negative IDs pass through. `as_i64().unwrap_or(0)` maps non-numeric to 0 (caught), but `-1` passes validation and triggers a silent no-op UPDATE.

## Findings

- **Source:** security-sentinel
- **Location:** `crates/mika-agent/src/tools/update_fact.rs:59`

## Proposed Solutions

### Option A: Change to `id <= 0` (Recommended)
- One-line fix: `if id <= 0 {`
- **Effort:** Tiny
- **Risk:** None

## Acceptance Criteria

- [ ] Negative IDs rejected with error message
- [ ] Test for negative ID added
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review of commit 3619d13 | Always validate both zero and negative for integer IDs |
