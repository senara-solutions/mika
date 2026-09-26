---
status: complete
priority: p3
issue_id: "546"
tags: [code-review, correctness, task-engine]
dependencies: []
---

# Reflection cron drifts by 1 hour across DST transitions

## Problem Statement

`reflection_cron_for_agent` converts local time to UTC using today's date to compute the offset. The resulting cron expression is stored and used until the next restart. For timezones with DST (e.g., America/New_York: UTC-5 winter, UTC-4 summer), the reflection will fire 1 hour early or late for roughly half the year.

Example: User wants reflection at 20:00 America/New_York.
- Winter: 20:00 EST = 01:00 UTC next day -> cron `0 0 1 * * *`
- Summer: 20:00 EDT = 00:00 UTC next day -> cron `0 0 0 * * *`
- If computed in winter and server runs through summer without restart, reflection fires at 21:00 EDT instead of 20:00 EDT.

## Findings

- **Source:** Architecture strategist + code simplicity reviewer
- **Location:** `crates/mika-agent/src/task_engine/mod.rs` lines 113-120
- **Evidence:** `chrono::Utc::now().with_timezone(&tz).date_naive()` pins the conversion to today's offset

## Proposed Solutions

### Option A: Recompute cron on each tick-loop scan (periodic)
- **Approach:** Every 60-tick DB scan in the engine, recompute the reflection cron and update if changed
- **Pros:** Self-corrects within ~60 seconds of DST transition
- **Cons:** Adds computation to the hot path (though chrono conversion is cheap)
- **Effort:** Medium
- **Risk:** Low

### Option B: Accept 1-hour drift, document it
- **Approach:** Add a comment noting DST drift is tolerable for daily reflections. Users in DST timezones will see reflections shift by 1 hour twice a year until next restart.
- **Pros:** Zero code change
- **Cons:** Imprecise for users who care about exact timing
- **Effort:** None
- **Risk:** None

### Option C: Use cron-with-timezone instead of UTC cron
- **Approach:** Store the cron in local time and convert at fire time
- **Pros:** Always correct regardless of DST
- **Cons:** Requires changes to the cron evaluation engine; the `cron` crate may or may not support timezone-aware evaluation
- **Effort:** Large
- **Risk:** Medium

## Technical Details

- **Affected files:** `crates/mika-agent/src/task_engine/mod.rs`, potentially `cron.rs`

## Recommended Action

Option B: Accept 1-hour drift, add a code comment documenting the known limitation. Daily reflections have no hard timing requirement and containers restart frequently.

## Acceptance Criteria

- [ ] Code comment added to `reflection_cron_for_agent` documenting DST drift limitation

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-07 | Created from code review | DST transitions shift UTC offset; static cron doesn't adapt |

## Resources

- PR branch: `feat/unified-task-engine`
