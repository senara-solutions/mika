---
title: "Reminders fire one day early due to missing timezone conversion"
category: logic-errors
date: 2026-03-30
tags: [timezone, reminders, cron, UTC, chrono-tz, prompt-engineering, task-engine]
issue: "#350"
---

# Reminders fire one day early due to missing timezone conversion

## Problem

Users reported that reminders set for a specific day (e.g., "remind me Thursday") would fire one day early. A user in UTC+8 saying "remind me Thursday" would get the reminder on Wednesday.

## Root Cause

The `create_reminder` tool accepted only UTC timestamps, and the system prompt told the LLM to provide `fire_at` in "ISO 8601 UTC." While the prompt showed the user's timezone (e.g., `User timezone: Asia/Singapore`), it never instructed the LLM to convert local time to UTC. The LLM would resolve "Thursday" relative to the UTC date (which could differ from the user's local date) and produce a UTC timestamp that was off by a day.

For recurring cron expressions the problem was worse: the LLM would write cron times in what it thought was UTC but was actually the user's local time, causing systematic drift.

The engine comparison logic (`next_fire_at <= now`) and `next_fire_from_cron()` were both correct — the bug was entirely in the prompt-to-tool interface where the LLM performed incorrect timezone arithmetic.

## Solution

Two-part fix that moves timezone conversion from the LLM to the engine:

### 1. Added `timezone` parameter to `create_reminder` tool

- Optional IANA timezone name (e.g., `Asia/Singapore`, `America/New_York`)
- One-shot: `fire_at` is interpreted as local naive datetime, converted to UTC by the engine
- Recurring: timezone stored in task `metadata` JSON; engine uses it for DST-aware rescheduling
- Backward compatible: UTC with `Z` suffix still works without `timezone`

### 2. Updated system prompt instructions

Changed from "provide fire_at in ISO 8601 UTC" to "pass the user's local time and their timezone; the tool handles UTC conversion automatically."

### Key files changed

- `crates/mika-agent/src/task_engine/cron.rs` — Added `next_fire_from_cron_tz(&Tz)`, `extract_timezone_from_metadata()`, `parse_timezone()` helpers
- `crates/mika-agent/src/tools/create_reminder.rs` — Added `timezone` parameter with `NaiveDateTime` parsing and `chrono_tz` conversion
- `crates/mika-agent/src/task_engine/engine.rs` — Reads timezone from task metadata during rescheduling
- `crates/mika-agent/src/tools/list_reminders.rs` — Shows local time when timezone metadata is available
- `crates/mika-agent/src/prompt.rs` — Updated reminder instructions

### DST handling

The `cron` crate's `Schedule::after()` accepts `DateTime<Tz>` (any timezone), so evaluating cron in the user's timezone naturally picks the correct local-time occurrence. Converting back to UTC at each specific future datetime captures the correct offset, handling DST transitions automatically (e.g., `America/New_York` 9am = 14:00 UTC in winter, 13:00 UTC in summer).

## Prevention

- **Prefer engine-side conversion over LLM arithmetic.** LLMs are unreliable at timezone math, especially near day boundaries and DST transitions. When a tool needs a UTC timestamp, accept local time + timezone and convert server-side.
- **Always include timezone in the tool schema** when the tool operates on user-meaningful times. Don't rely on the system prompt to instruct the LLM to convert.
- **Store timezone metadata with recurring tasks** so rescheduling remains correct across DST transitions. A static UTC offset baked into a cron expression is wrong for half the year.
- **Test across timezone boundaries.** The test suite now includes: UTC+8 (Singapore), UTC-5/UTC-4 (New York with DST), day boundary crossings, and DST spring-forward transitions.
