---
title: Fix QA HOLD-2 and HOLD-3 on timezone reminder PR
type: fix
status: completed
date: 2026-03-30
---

# Fix QA HOLD-2 and HOLD-3 on timezone reminder PR

## Context

PR #351 (mika#350) adds timezone support to reminders. QA reviewed and issued a HOLD with three findings. HOLD-1 (CI) is resolved. HOLD-2 and HOLD-3 are code-level issues that need fixing before merge.

## HOLD-2: Silent timezone discard on Z-suffix fire_at

**File:** `crates/mika-agent/src/tools/create_reminder.rs:186-199`

**Problem:** When `timezone` is provided but `fire_at` is already offset-aware (e.g. `2099-12-31T23:59:59Z`), `NaiveDateTime::parse_from_str` fails (because of the `Z`), and the code falls through to `DateTime::parse_from_rfc3339` which silently ignores the `timezone` parameter. The user gets no indication their timezone was discarded.

**Fix:** Return an error when both `timezone` and an offset-aware `fire_at` are provided. The error message should tell the user to either drop the timezone parameter or use a naive datetime format.

**Implementation:**
- In the `Err(_)` arm at line 186, before falling through to RFC 3339, check if `fire_at` parses as RFC 3339. If it does and `timezone` was provided, return a `ToolOutput::error` explaining the conflict.
- Update test `test_create_reminder_timezone_with_rfc3339_fallback` (line 670) to assert `is_error` instead of `!is_error`, and check the error message mentions the conflict.

## HOLD-3: Silent DB error fallback in engine rescheduling

**File:** `crates/mika-agent/src/task_engine/engine.rs:500-505`

**Problem:** `db.get_task(&task_id).await` uses `_ => None` which swallows `Err(e)` — if the DB call fails, a timezone-configured recurring reminder silently degrades to UTC-only rescheduling.

**Fix:** Add a `warn!` log on the `Err` arm so the fallback is visible in logs.

**Implementation:**
- Replace `_ => None` with explicit match arms: `Ok(None) => None` and `Err(e) => { warn!(...); None }`.
- The second `_ => false` at line 570 is lower risk (expiry check, not timezone) — leave it as-is per QA scope.

## Acceptance Criteria

- [x] Providing `timezone: "Asia/Singapore"` with `fire_at: "2099-12-31T23:59:59Z"` returns an error
- [x] Error message explains the conflict and suggests alternatives
- [x] `test_create_reminder_timezone_with_rfc3339_fallback` updated to expect error
- [x] `engine.rs` DB error arm logs `warn!` with task_id and error
- [x] `cargo test` passes
- [x] `cargo clippy` clean

## MVP

### create_reminder.rs (line 186 Err arm)

```rust
Err(_) => {
    // fire_at wasn't a naive datetime — check if it's offset-aware
    match chrono::DateTime::parse_from_rfc3339(fire_at) {
        Ok(_) => {
            // Conflict: user provided both timezone and offset-aware fire_at
            return Ok(ToolOutput::error(
                "Conflicting timezone info: 'fire_at' already includes a UTC offset (e.g. 'Z' or '+08:00'), \
                 but 'timezone' was also provided. Either remove the 'timezone' parameter, \
                 or use a local datetime format like '2026-04-02T09:00:00' with 'timezone'.",
            ));
        }
        Err(_) => {
            return Ok(ToolOutput::error(
                "Invalid datetime. With timezone, use local format like '2026-04-02T09:00:00'. \
                 Without timezone, use ISO 8601 UTC like '2026-04-02T01:00:00Z'.",
            ));
        }
    }
}
```

### engine.rs (line 500-505)

```rust
let parsed_tz = match db.get_task(&task_id).await {
    Ok(Some(task)) => {
        extract_timezone_from_metadata(task.metadata.as_deref())
            .and_then(|tz_str| parse_timezone(&tz_str).ok())
    }
    Ok(None) => None,
    Err(e) => {
        warn!(task_id = %task_id, error = %e, "failed to read task for timezone metadata, falling back to UTC");
        None
    }
};
```
