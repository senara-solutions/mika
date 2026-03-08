---
status: pending
priority: p1
issue_id: "578"
tags: [code-review, quality]
dependencies: []
---

# No Tests for Dashboard Endpoints or Queries

## Problem Statement
The PR adds 12 new Database methods, 12 AsyncDatabase wrappers, 9 HTTP handlers, and helper functions (`strip_base64_images`, `TimelineFilters::to_sql()`) with zero test coverage. The existing codebase has 943 tests with inline `#[cfg(test)] mod tests` convention, which this PR breaks.

## Findings
- **Source:** Architecture Strategist agent
- **Severity:** HIGH — dynamic SQL and heuristic functions are fragile without tests
- **Location:** `crates/mika-agent/src/db.rs` (dashboard queries), `crates/mika-agent/src/server/dashboard.rs` (handlers + helpers)

## Proposed Solutions

### Option A: Unit tests for critical helpers + integration tests for handlers
- Unit tests for: `TimelineFilters::to_sql()` (various filter combinations), `strip_base64_images` (boundary conditions at 1000/50000 bytes), `resolve_pagination` (overflow)
- Integration tests for 2-3 key handlers using existing `test_app()` infrastructure
- **Effort:** Medium
- **Risk:** Low

## Acceptance Criteria
- [ ] `TimelineFilters::to_sql()` tested with 0, 1, and all filters
- [ ] `strip_base64_images` tested at boundary conditions
- [ ] `resolve_pagination` overflow tested
- [ ] At least one handler integration test

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from code review | Architecture Strategist flagged test gap |

## Resources
- PR #89
