---
title: "Silent timezone discard on offset-aware fire_at + silent DB error UTC fallback"
category: logic-errors
date: 2026-03-30
tags: [reminders, timezone, silent-failure, error-handling, create_reminder, task-engine]
related:
  - docs/solutions/logic-errors/reminders-fire-one-day-early-timezone-gap.md
  - https://github.com/senara-solutions/mika/pull/351
  - https://github.com/senara-solutions/mika/issues/350
---

# Silent timezone discard on offset-aware fire_at + silent DB error UTC fallback

## Problem

Two silent failure modes in the timezone-aware reminder system (PR #351):

1. **create_reminder**: When `timezone` is provided alongside an offset-aware `fire_at` (e.g. `2099-12-31T23:59:59Z`), `NaiveDateTime` parsing fails on the `Z` suffix, and the code falls through to RFC 3339 parsing which silently ignores the `timezone` parameter. The user gets a "scheduled" confirmation with no indication their timezone was discarded.

2. **engine.rs rescheduling**: `db.get_task()` uses `_ => None` which swallows DB errors. A recurring reminder with timezone metadata silently degrades to UTC-only rescheduling if the DB read fails.

## Root Cause

Both are instances of the same anti-pattern: catch-all match arms (`_ =>`) that silently discard error information, making failures invisible.

## Solution

### HOLD-2: Conflict detection in create_reminder

In the `Err(_)` arm after `NaiveDateTime` parsing fails, check if `fire_at` is a valid RFC 3339 datetime. If it is and `timezone` was provided, return an explicit error explaining the conflict:

```rust
Err(_) => {
    match chrono::DateTime::parse_from_rfc3339(fire_at) {
        Ok(_) => {
            return Ok(ToolOutput::error(
                "Conflicting timezone info: 'fire_at' already includes a UTC offset...",
            ));
        }
        Err(_) => {
            return Ok(ToolOutput::error("Invalid datetime..."));
        }
    }
}
```

### HOLD-3: Explicit error logging in engine rescheduling

Replace `_ => None` with explicit match arms so DB errors are logged:

```rust
Ok(None) => None,
Err(e) => {
    warn!(task_id = %task_id, error = %e,
        "failed to read task for timezone metadata, falling back to UTC");
    None
}
```

## Prevention

- Avoid `_ => None` or `_ => false` match arms on `Result` types — always handle `Err` explicitly, at minimum with a `warn!` log.
- When a tool accepts multiple parameters that can conflict (e.g. `timezone` + offset-aware datetime), validate the combination early and return a clear error.
- QA review skill (`mika-qa`) correctly identified both patterns — the `qa-review` skill's diff analysis catches silent error swallowing.
