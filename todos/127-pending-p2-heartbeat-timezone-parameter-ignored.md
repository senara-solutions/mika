---
status: pending
priority: p2
issue_id: "127"
tags: [code-review, correctness]
dependencies: []
---

# count_heartbeat_sends_today Ignores Timezone Parameter

## Problem Statement

The `count_heartbeat_sends_today` method in `db.rs` accepts a `timezone` parameter but the SQL query uses `date('now')` (UTC) instead of computing the local date boundary. This means the "3 per day" rate limit uses UTC days, not the customer's local days. A customer at UTC-8 could get heartbeats at 11pm and 1am local time (different UTC days) when they should be rate-limited.

## Findings

- **Source:** code-simplicity-reviewer
- **Location:** `crates/mika-agent/src/db.rs` — `count_heartbeat_sends_today` method
- **Evidence:** Parameter `_timezone: &str` accepted but SQL uses `date('now')` — UTC-based

## Proposed Solutions

### Option 1: Compute local date boundary in Rust, pass to SQL
- **Pros**: Correct timezone handling, matches user expectations
- **Cons**: Slightly more complex
- **Effort**: Small
- **Risk**: Low

### Option 2: Remove timezone parameter, document UTC-based behavior
- **Pros**: Honest API, no misleading parameter
- **Cons**: Rate limiting still wrong for non-UTC users
- **Effort**: Trivial
- **Risk**: Low

## Recommended Action

Option 1 — compute the local start-of-day timestamp using chrono-tz (already a dependency) and use it in the SQL query.

## Technical Details

- **Affected Files**: `crates/mika-agent/src/db.rs`
- **Database Changes**: None

## Acceptance Criteria

- [ ] Heartbeat daily rate limit uses customer's local day boundaries
- [ ] Tests verify correct behavior across timezone boundaries
- [ ] All existing tests pass

## Work Log

### 2026-02-24 - Identified during PR #5 review
**By:** code-simplicity-reviewer

## Resources

- PR #5: Phase 2 Container HTTP Server
