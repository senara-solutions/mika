---
status: complete
priority: p3
issue_id: 230
tags: [code-review, simplicity, performance, slash-commands]
dependencies: []
---

# Simplify pending_command Pattern and Unnecessary Clone

## Problem Statement

Two related simplification opportunities in the tick/command dispatch flow:

1. `pending_command` exists because `send_message()` is sync but `dispatch()` is async. If `send_message()` were async (or command dispatch moved to a different path), the indirection could be eliminated.

2. `pending_response.clone()` in `tick()` creates an unnecessary copy of the response string. The value can be taken with `Option::take()` instead.

## Findings

**Source:** Code Simplicity Reviewer + Performance Oracle

**Locations:**
- `crates/mika-cli/src/tui/app.rs` — `pending_command` field and queue/dequeue pattern
- `crates/mika-cli/src/tui/app.rs` — `pending_response.clone()` in `tick()`

## Proposed Solutions

### Solution A: Fix the clone, keep the pattern (Recommended)
- Replace `pending_response.clone()` with `pending_response.take()` (or `Option::take()`)
- Keep `pending_command` pattern — it's clean and the sync/async boundary is real
- **Pros:** Quick fix, eliminates unnecessary allocation
- **Cons:** Doesn't eliminate the indirection
- **Effort:** Small
- **Risk:** None

### Solution B: Eliminate pending_command entirely
- Make the main event loop handle commands directly
- **Pros:** Simpler flow
- **Cons:** Requires restructuring event loop, higher risk
- **Effort:** Medium
- **Risk:** Medium

## Recommended Action

Solution A — fix the clone, keep the pattern. The pending_command pattern is architecturally sound.

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/app.rs`

## Acceptance Criteria

- [ ] `pending_response.clone()` replaced with `take()` or equivalent
- [ ] No unnecessary string allocation in tick()

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from code review | Simplicity + performance reviewers flagged |

## Resources

- PR branch: `feat/slash-commands`
