---
status: pending
priority: p2
issue_id: "076"
tags: [code-review, performance, architecture]
dependencies: []
---

# Make reminder recovery non-blocking on startup

## Problem Statement
`ReminderScheduler::recover()` fires past-due reminders sequentially, each requiring Claude API calls. With N past-due reminders after an outage, startup blocks for N * (2-5s API latency). 10 past-due reminders could block CLI for 50-500 seconds.

## Findings
- scheduler.rs:44-71 — sequential `for reminder in &past_due` loop
- cli.rs:66-75 — scheduler.recover() called before CLI prompt shown
- Each run_silent_agent makes 1+ Claude API calls with 5min max timeout
- K8s health checks would fail during extended recovery

## Proposed Solutions
### Option 1: Background spawn after startup
Move recovery after CLI prompt / HTTP listener start.
**Effort:** 15 minutes | **Risk:** Low

### Option 2: Add staleness cutoff + batch cap
Process max 5 reminders, expire reminders > 24h old.
**Effort:** 30 minutes | **Risk:** Low

## Acceptance Criteria
- [ ] CLI prompt appears before reminder recovery starts
- [ ] Past-due recovery does not block container readiness

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
