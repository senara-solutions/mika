---
title: "fix: Reminders deliver one day earlier than expected"
type: fix
status: completed
date: 2026-03-30
issue: "#350"
---

# fix: Reminders deliver one day earlier than expected

## Overview

Reminders set through the Mika assistant fire one day earlier than expected. A user who says "remind me Thursday" gets the reminder on Wednesday. The root cause is a timezone conversion gap: the `create_reminder` tool accepts only UTC timestamps, the system prompt shows the user's timezone but never instructs the LLM to convert local time to UTC, and the LLM frequently fails to perform the conversion correctly — especially when the UTC date differs from the local date.

## Problem Statement

The reminder pipeline operates entirely in UTC:
- `create_reminder` tool accepts `fire_at` as "ISO 8601 datetime (UTC)" and `cron_expr` with examples like "9am UTC"
- `next_fire_from_cron()` evaluates cron expressions in UTC via the `cron` crate
- Task engine dispatches based on UTC comparison (`next_fire_at <= now`)

The system prompt (`prompt.rs:149-155`) shows:
```
UTC: 2026-03-30T14:00:00Z
User timezone: Asia/Singapore
```

But the reminder instruction (`prompt.rs:393`) says only:
```
You can create reminders with create_reminder: one-shot (fire_at in ISO 8601 UTC) or periodic (cron_expr, 6-field cron with seconds first)
```

**No instruction tells the LLM to convert the user's local time to UTC.** The LLM sees "Thursday" and often produces a UTC timestamp for Thursday — but if the user's local date is already Thursday while UTC is still Wednesday, the reminder fires a day early from the user's perspective.

For cron expressions the problem is worse: "every Thursday at 9am" for a user in UTC+8 should be `0 0 1 * * 4` (1am UTC Thursday = 9am SGT Thursday), but the LLM typically produces `0 0 9 * * 4`.

## Proposed Solution

A two-part fix that makes timezone conversion **engine-side** rather than relying on LLM arithmetic:

### Part 1: Add `timezone` parameter to `create_reminder` tool

Add an optional `timezone` field (IANA timezone name, e.g., `Asia/Singapore`). When provided:
- **One-shot (`fire_at`):** Accept offset-naive local datetime (e.g., `2026-04-02T09:00:00`), convert to UTC using the provided timezone. Continue accepting `Z`-suffixed UTC for backward compatibility.
- **Recurring (`cron_expr`):** Store timezone in task `metadata` JSON. At rescheduling time, compute next fire in the user's local timezone, then convert to UTC. This handles DST transitions correctly.

### Part 2: Update system prompt instructions

Update the reminder instruction in `prompt.rs` to tell the LLM to:
- Pass the user's local time (not UTC) to `create_reminder`
- Include the `timezone` parameter from the user's configured timezone
- For cron expressions, write the schedule in the user's local time

This ensures the LLM does the easy part (parsing natural language into local time) and the engine does the hard part (timezone conversion).

## Technical Approach

### `create_reminder.rs` — Tool changes

**Add `timezone` parameter to schema** (after line 43):
```rust
"timezone": {
    "type": "string",
    "description": "IANA timezone name (e.g. 'Asia/Singapore', 'America/New_York'). When provided, fire_at is interpreted as local time in this timezone. For cron expressions, the schedule is evaluated in this timezone."
}
```

**One-shot path** (lines 109-136): When `timezone` is provided and `fire_at` lacks an offset:
1. Parse `fire_at` as `NaiveDateTime`
2. Use `chrono_tz::Tz` to convert to `DateTime<Tz>`
3. Convert to `DateTime<Utc>` for storage
4. Fall through to existing `parse_from_rfc3339` path when `fire_at` has an offset or `Z` suffix (backward compat)

**Recurring path** (lines 66-108): When `timezone` is provided:
1. Store `{"timezone": "..."}` in the task's `metadata` JSON field
2. Use timezone-aware next-fire computation (see `cron.rs` changes below)
3. Display confirmation in local time

**Validation:** Use `chrono_tz::Tz::from_str()` — reject with clear error message if invalid.

### `cron.rs` — Timezone-aware cron evaluation

Add `next_fire_from_cron_tz(expr, after_utc, timezone)`:
1. Parse the IANA timezone with `chrono_tz`
2. Convert `after_utc` to local time in the given timezone
3. Use `schedule.after(&local_dt)` to get next fire in local time
4. Convert back to UTC for storage

This correctly handles DST: the `cron` crate's `Schedule::after()` accepts `DateTime<Tz>` (any timezone), so it naturally picks the right local-time occurrence. The conversion back to UTC at the specific future datetime captures the correct offset.

### `engine.rs` — Rescheduling with timezone

In `fire_task()` (line 480-498), when rescheduling a recurring task:
1. Read `timezone` from task `metadata` (if present)
2. Call `next_fire_from_cron_tz()` if timezone exists, else fall back to existing `next_fire_from_cron()`
3. No change to dispatch logic — it still compares UTC timestamps

### `prompt.rs` — Instruction update

Replace line 393 with explicit timezone-aware instructions:
```
When creating reminders, ALWAYS pass the user's local time and timezone:
- One-shot: fire_at in the user's local time (e.g. '2026-04-02T09:00:00'), timezone from the User timezone shown above
- Periodic: cron_expr in the user's local time (e.g. '0 0 9 * * 1' for Monday 9am local), timezone from the User timezone shown above
- If no User timezone is shown above, use UTC (fire_at with Z suffix, no timezone parameter)
```

### `list_reminders.rs` — Local time display

When presenting reminders, include timezone context in the tool output so the LLM can present local times:
- Read `timezone` from task `metadata` (if available)
- Append timezone label to the displayed time (e.g., `2026-04-02 09:00 Asia/Singapore`)
- Fall back to `UTC` label when no timezone stored

## Acceptance Criteria

- [x] `create_reminder` accepts optional `timezone` (IANA name) parameter — `create_reminder.rs`
- [x] One-shot reminders with `timezone` + naive local datetime are correctly converted to UTC — `create_reminder.rs`
- [x] One-shot reminders with `Z` suffix continue to work without `timezone` (backward compat) — `create_reminder.rs`
- [x] Recurring reminders store timezone in `metadata` — `create_reminder.rs`
- [x] `next_fire_from_cron_tz()` computes correct UTC fire times across DST boundaries — `cron.rs`
- [x] Engine rescheduling reads timezone from metadata and uses timezone-aware cron — `engine.rs`
- [x] System prompt instructs LLM to pass local time + timezone — `prompt.rs`
- [x] `list_reminders` shows timezone context when available — `list_reminders.rs`
- [x] Invalid timezone is rejected with clear error — `create_reminder.rs`
- [x] Existing UTC-only reminders continue to work (no migration needed) — `engine.rs`
- [x] Confirmation message shows both local and UTC time when timezone is provided — `create_reminder.rs`
- [x] Tests cover: timezone conversion, DST boundary, backward compat, invalid timezone, cron rescheduling with timezone — `create_reminder.rs`, `cron.rs`

## Dependencies & Risks

- **`chrono-tz`** is already a dependency in both `mika-agent` and `mika-cli` Cargo.toml — no new deps needed
- **`metadata` JSON column** already exists on the `tasks` table (schema v14) — no migration needed
- **Backward compatibility:** Existing reminders have no timezone in metadata; the rescheduling path falls back to UTC-only cron evaluation. No data migration required.
- **DST edge cases:** The `cron` crate handles ambiguous/skipped local times (spring forward / fall back) — `Schedule::after()` with a `DateTime<Tz>` does the right thing. Document the behavior: skipped times fire at the next valid time; ambiguous times use the earlier offset.
- **LLM compliance:** Even with the tool-side fix, the LLM might not always pass the timezone parameter. The prompt instructions mitigate this, but it's not guaranteed. The fix still improves the situation because when the LLM does pass timezone, the conversion is correct (vs. current state where it's always wrong).

## Sources

- Issue: [#350](https://github.com/senara-solutions/mika/issues/350)
- `crates/mika-agent/src/tools/create_reminder.rs` — tool definition and execution
- `crates/mika-agent/src/tools/list_reminders.rs` — reminder display
- `crates/mika-agent/src/task_engine/cron.rs` — cron next-fire computation
- `crates/mika-agent/src/task_engine/engine.rs:480-498` — recurring task rescheduling
- `crates/mika-agent/src/prompt.rs:149-155,393` — system prompt time section and reminder instructions
- `docs/solutions/database-issues/sqlite-datetime-format-mismatch.md` — prior timestamp format bug
- `docs/solutions/runtime-errors/reminders-never-fire-at-scheduled-time.md` — reminder dispatch context
