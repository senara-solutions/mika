---
title: "fix(tui): Add background task running indicator to TUI footer"
type: fix
status: completed
date: 2026-04-04
---

# fix(tui): Add background task running indicator to TUI footer

## Overview

The TUI footer shows a `[N tasks]` badge for user-visible reminders but has no indicator for active long-running background tasks (like claude-pilot runs). After PR #429 removed dashboard status polling, the user expected to see background task activity — but that was never displayed. The root cause is a missing feature, not a regression.

## Problem Statement

`get_user_visible_tasks()` in `db.rs:3311` intentionally excludes `trigger_type = 'callback'` tasks (see comment at line 3307-3309). Long-running tasks spawned by exec handlers (e.g., claude-pilot via `self-dev` skill) are callback tasks — they never appeared in the `[N tasks]` badge. The task count polling code (app.rs:968-978) is still functional and was NOT removed by PR #429.

The user's perception that "the indicator stopped updating" is because the dashboard dot (green/red) was the only visual signal of background activity, and its polling was removed. There is genuinely no way to see running background tasks in the TUI.

## Proposed Solution

Add a separate `[N running]` badge in the TUI footer that shows the count of active callback tasks. This requires:

1. A new DB query for active background tasks
2. A new state field on the App struct
3. Polling alongside existing task count polling
4. Rendering a distinct badge in the footer

## Technical Approach

### 1. New DB Query — `get_active_background_task_count()`

**File:** `crates/mika-agent/src/db.rs`

Add a COUNT-only query (no need to fetch full Task rows):

```rust
/// Count active background tasks (long-running callback tasks that are pending or in-progress).
/// Used by TUI footer badge. Complements `get_user_visible_tasks()` which intentionally
/// excludes callback tasks.
pub fn get_active_background_task_count(&self, agent_id: &str) -> Result<usize> {
    let count: i64 = self.conn.query_row(
        "SELECT COUNT(*) FROM tasks
         WHERE agent_id = ?1
           AND trigger_type = 'callback'
           AND action_type = 'resume_agent'
           AND status IN ('pending', 'in_progress')",
        params![agent_id],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}
```

Key design decisions:
- Filter on both `trigger_type = 'callback'` AND `action_type = 'resume_agent'` for consistency with `get_undelivered_callback_tasks()` (db.rs:3345)
- Use COUNT query — more efficient than fetching full rows when only the count is needed
- No index needed — tasks table is small per agent, polled every ~5s

Add corresponding async wrapper in `AsyncDatabase`.

### 2. New App State Field

**File:** `crates/mika-cli/src/tui/app.rs`

```rust
// Existing field (line 539):
pub pending_task_count: usize,
// New field:
pub active_background_task_count: usize,
```

Initialize to `0` in both `App::new()` and `App::new_team()`.

**`/clear` behavior:** Do NOT reset `active_background_task_count` on `/clear`. Background tasks are agent-scoped (not session-scoped) — they persist across sessions. Resetting would cause a brief flicker (0 for up to 5s until next poll). This is different from `pending_task_count` which IS reset because reminders are conceptually tied to session context.

### 3. Polling in `tick()`

**File:** `crates/mika-cli/src/tui/app.rs` (after line 978)

Combine with the existing task count polling block to minimize DB round-trips:

```rust
// Background task count polling: refresh every ~5s for footer badge.
if !self.is_team_mode()
    && self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS)
    && let Ok(count) = self.db.get_active_background_task_count().await
{
    if count != self.active_background_task_count {
        self.active_background_task_count = count;
        self.needs_redraw = true;
    }
}
```

### 4. Footer Badge Rendering

**File:** `crates/mika-cli/src/tui/ui.rs` (after line 1054)

Add a new badge after the existing `[N tasks]` badge:

```rust
// Active background task badge
if app.active_background_task_count > 0 {
    s.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));
    s.push(Span::styled(
        format!("[{} running]", app.active_background_task_count),
        Style::default().fg(Color::Yellow),
    ));
}
```

- Color: `Yellow` — implies "in progress" semantics, distinct from Cyan (informational)
- Text: `[N running]` — "running" works for both singular and plural, avoids the `[1 tasks]` awkwardness
- Position: after `[N tasks]` badge, before scroll indicator

## Acceptance Criteria

- [x] New `get_active_background_task_count()` method on `Database` with `agent_id` scoping
- [x] Corresponding `get_active_background_task_count()` on `AsyncDatabase`
- [x] `active_background_task_count: usize` field on `App` struct, initialized to 0
- [x] Polling in `tick()` every ~5s (reuses `POLL_INTERVAL_TICKS`), skipped in team mode
- [x] `[N running]` badge in Yellow in TUI footer when count > 0
- [x] `/clear` does NOT reset `active_background_task_count`
- [x] Unit test for `get_active_background_task_count()` — counts only callback/resume_agent tasks in pending/in_progress
- [x] Unit test confirming `/clear` preserves `active_background_task_count`
- [x] `cargo test` passes, `cargo clippy` clean

## Non-Goals

- Changing the semantics of `[N tasks]` badge (remains reminders-only)
- Showing task labels/names in the badge (count is sufficient)
- Team mode support (team engine has its own progress via `TeamEvent`)
- Showing delegate agent callbacks in orchestrator TUI (callbacks are agent_id-scoped)

## Sources

- Issue: #431
- Related PR: #429 (dashboard polling removal)
- Related PR: #425 (dashboard check-once-at-startup)
- `crates/mika-agent/src/db.rs:3304-3326` — `get_user_visible_tasks()` (excludes callbacks)
- `crates/mika-agent/src/db.rs:3345-3362` — `get_undelivered_callback_tasks()` (query pattern reference)
- `crates/mika-cli/src/tui/app.rs:539` — `pending_task_count` field
- `crates/mika-cli/src/tui/app.rs:968-978` — task count polling (still functional)
- `crates/mika-cli/src/tui/ui.rs:1047-1054` — existing task badge rendering
- `docs/solutions/ux-improvements/tui-dashboard-status-polling-removal.md` — #429 documented learnings
- `docs/solutions/architecture-patterns/callback-tui-delivery-polling.md` — TUI polling architecture
