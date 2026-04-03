---
title: "Remove TUI dashboard status polling — check once at startup"
category: ux-improvements
date: 2026-04-03
tags: [tui, polling, dashboard, server-logs, performance]
issue: 425
---

# Remove TUI Dashboard Status Polling

## Problem

The TUI footer polled `GET /api/v1/dashboard/status` every 5 seconds (167 ticks at 30ms) to keep the `[start]`/`[stop]`/`[open]` dashboard buttons current. This flooded server logs with INFO-level 200 responses:

```
INFO response status=200 method=GET path=/api/v1/dashboard/status latency=58µs
```

The dashboard state (enabled/disabled) rarely changes mid-session — it only changes when someone explicitly toggles it.

## Root Cause

The polling was implemented when the dashboard toggle feature was added (`crates/mika-cli/src/tui/app.rs` tick handler). The design assumed the TUI needed to detect external state changes, but in practice the only state changes come from the TUI's own footer buttons, which already update `dashboard_running` optimistically on success.

## Solution

1. **Removed the polling block** from `tick()` in `app.rs` (was lines 989-996)
2. **Added a one-time startup check** in `chat.rs` — both `run()` (regular chat) and `run_team()` (team mode) call `is_dashboard_running()` once before the event loop starts
3. **Preserved optimistic updates** — the footer `[start]`/`[stop]` click handler already sets `dashboard_running` immediately on API success (line 885 in `app.rs`)

Key detail: `auth_token()` inside `is_dashboard_running()` returns `None` instantly when no `MIKA_INTERNAL_TOKEN` or `MIKA_DASHBOARD_TOKEN` is configured, so CLI-only users (no server) experience zero startup latency from this check.

## Prevention

When adding polling to a UI, consider whether the polled state is event-driven (changes from user action) vs. externally-driven (changes from outside the process). Event-driven state should use optimistic updates from the action handler, not polling. Reserve periodic polling for data that changes asynchronously (e.g., callback task delivery, cross-channel messages).

The `POLL_INTERVAL_TICKS` constant (167 ticks = ~5s) is still used by three legitimately async polling consumers in `tick()`: cross-channel messages, task counts, and callback delivery.
