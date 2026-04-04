---
title: "TUI background task running indicator"
category: ux-improvements
date: 2026-04-04
tags: [tui, footer, polling, callback-tasks, background-tasks]
related_issues: ["#431", "#429", "#425"]
---

# TUI Background Task Running Indicator

## Problem

After PR #429 removed dashboard status polling from the TUI `tick()` loop, users reported that the "background task indicator" stopped updating (#431). Investigation revealed this was a **missing feature, not a regression**: the existing `[N tasks]` footer badge only shows user-visible reminders (`send_message`/`resume_agent` with `trigger_type NOT IN ('callback')`). Long-running callback tasks (like claude-pilot runs) were intentionally excluded from `get_user_visible_tasks()` and had no TUI indicator.

The user's perception was driven by the dashboard dot (green/red) being the only visual signal of background activity — once its polling was removed, there was genuinely no way to see running background tasks.

## Root Cause

`get_user_visible_tasks()` in `db.rs` filters `trigger_type NOT IN ('callback')` by design (see comment at db.rs:3307-3309). Callback tasks are system-internal tasks created by long-running exec handlers. They were never meant to appear in the `[N tasks]` badge. There was simply no separate indicator for them.

## Solution

Added a new `[N running]` badge (Yellow) in the TUI footer, separate from the existing `[N tasks]` badge (Cyan):

1. **New DB query** — `get_active_background_task_count(agent_id)`: COUNT-only query filtering `trigger_type = 'callback' AND action_type = 'resume_agent' AND status IN ('pending', 'in_progress')`. Consistent with `get_undelivered_callback_tasks()` filter pattern.

2. **New App state field** — `active_background_task_count: usize`: Agent-scoped (not session-scoped), intentionally NOT reset on `/clear` since background tasks persist across sessions.

3. **Polling in tick()** — Same `POLL_INTERVAL_TICKS` (~5s) cadence as existing task count polling. Skipped in team mode.

4. **Footer rendering** — `[N running]` in `Color::Yellow` after the `[N tasks]` badge, before the scroll indicator.

## Key Design Decision: /clear Behavior

`pending_task_count` IS reset on `/clear` (reminders are session-scoped). `active_background_task_count` is NOT reset (background tasks are agent-scoped — they survive session changes). This prevents a brief flicker (0 for up to 5s until next poll) and correctly reflects that background processes are independent of the chat session.

## Prevention

- When removing polling mechanisms, audit all consumers sharing the same interval constant (`POLL_INTERVAL_TICKS`). The `tui-dashboard-status-polling-removal.md` solution already documented the three remaining consumers — this fix adds a fourth.
- New TUI badges should follow the established pattern: state field → polling in `tick()` → conditional rendering in `draw_footer()`.
- Agent-scoped vs session-scoped state should be explicitly documented in field comments and tested in the `/clear` test.

## Files Changed

- `crates/mika-agent/src/db.rs` — `get_active_background_task_count()`
- `crates/mika-agent/src/async_db.rs` — Async wrapper
- `crates/mika-cli/src/tui/app.rs` — Field, initialization, polling
- `crates/mika-cli/src/tui/ui.rs` — Badge rendering
- `crates/mika-cli/src/tui/commands/handlers.rs` — `/clear` preservation test
