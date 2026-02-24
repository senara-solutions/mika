---
status: pending
priority: p2
issue_id: "084"
tags: [code-review, performance, database]
dependencies: []
---

# Wire up heartbeat_sends pruning to prevent unbounded growth

## Problem Statement
`prune_old_heartbeat_sends()` exists in db.rs but is never called from production code. Every heartbeat inserts a row with no cleanup. The table grows linearly, degrading rate-limiting query performance over time.

## Findings
- db.rs:799-804 — prune method exists but only called in tests
- No caller in scheduler.rs, agent.rs, or cli.rs
- count_heartbeat_sends_today/last_hour queries scan entire table

## Proposed Solutions
### Option 1: Call pruning periodically
Call `prune_old_heartbeat_sends(7)` once on startup after recovery.
**Effort:** 5 minutes | **Risk:** Low

### Option 2: Prune after each heartbeat
Call after each `record_heartbeat_send()`. One extra DELETE per heartbeat.
**Effort:** 5 minutes | **Risk:** Low

## Acceptance Criteria
- [ ] Pruning wired into production code path
- [ ] heartbeat_sends does not grow unboundedly

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
