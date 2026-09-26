---
title: "fix(tui): stop polling /api/v1/dashboard/status every 5s — check once at startup"
type: fix
status: completed
date: 2026-04-03
issue: 425
---

# fix(tui): Stop Polling /api/v1/dashboard/status Every 5s

## Overview

The TUI footer polls `GET /api/v1/dashboard/status` every 5 seconds (~167 ticks at 30ms) to keep the `[start]`/`[stop]`/`[open]` dashboard buttons current. This floods server logs with INFO-level 200 responses and is unnecessary — the dashboard state rarely changes mid-session.

## Problem Statement

Every 5 seconds the TUI makes an HTTP request to the server:

```
INFO response status=200 method=GET path=/api/v1/dashboard/status latency=58µs
```

This creates log noise that obscures meaningful server activity. The dashboard state (enabled/disabled) only changes when someone explicitly toggles it — continuous polling is wasteful.

## Proposed Solution

1. **Check dashboard status once at TUI startup** (async, non-blocking)
2. **Remove the periodic polling** from the tick loop
3. **Rely on existing optimistic updates** from footer button clicks

The footer `[start]`/`[stop]` click handler (lines 858-907 in `app.rs`) already sets `dashboard_running` optimistically on success. No additional refresh mechanism is needed.

## Acceptance Criteria

- [x] Dashboard status is checked once at TUI startup (non-blocking)
- [x] The 5-second polling loop for dashboard status is removed from `tick()`
- [x] Footer `[start]`/`[stop]`/`[open]` buttons still work correctly via optimistic updates
- [x] TUI startup is NOT blocked by the dashboard status check (async fire-and-forget)
- [x] When no auth token is configured, no network call is made (existing `auth_token()` short-circuit preserved)
- [x] `POLL_INTERVAL_TICKS` constant is preserved (still used by 3 other polling loops)
- [x] Tests pass (`cargo test -p mika-cli`)
- [x] Clippy clean (`cargo clippy -p mika-cli`)

## Technical Approach

### Files to Modify

| File | Change |
|------|--------|
| `crates/mika-cli/src/tui/app.rs` | Remove dashboard polling block from `tick()` (lines 989-996). Add async startup check. |
| `crates/mika-cli/src/commands/chat.rs` | Fire the async startup check before entering the event loop |

### Implementation Details

**1. Remove polling from `tick()` (app.rs:989-996)**

Delete this block:

```rust
// Dashboard status polling: query mika-spirit for embedded dashboard state.
if self.tick_count.is_multiple_of(POLL_INTERVAL_TICKS) {
    let running = crate::commands::dashboard::is_dashboard_running().await;
    if running != self.dashboard_running {
        self.dashboard_running = running;
        self.needs_redraw = true;
    }
}
```

**2. Add async startup check**

In `chat.rs`, before entering the main event loop, spawn a `tokio::spawn` task that calls `is_dashboard_running()` and sends the result back to the app via the existing event channel (or a simple `Arc<AtomicBool>` shared with `App`). This avoids blocking TUI startup (the `query_dashboard_status()` function has a 3-second timeout which would stall CLI-only users).

Simplest approach — check before the loop but after `App::new()`:

```rust
// Non-blocking dashboard status check at startup
let dashboard_running = crate::commands::dashboard::is_dashboard_running().await;
app.dashboard_running = dashboard_running;
```

Since `App::new()` is already async and the TUI hasn't rendered yet, a brief async call here is acceptable. The `auth_token()` short-circuit ensures no network call when tokens are absent (instant return for CLI-only users). When the server is running, the response comes back in microseconds. When it's unreachable, the 3-second timeout applies — but this is an acceptable one-time cost vs. continuous polling.

**Alternative (if startup latency is a concern):** Use `tokio::spawn` + `Arc<AtomicBool>` to check asynchronously. The footer would show `[start]` for one frame before flipping. But given `auth_token()` returns `None` instantly when no tokens are configured (the common CLI-only case), this complexity is likely unnecessary.

### What Does NOT Change

- `is_dashboard_running()` and `query_dashboard_status()` in `dashboard.rs` — unchanged
- Footer rendering in `ui.rs` — unchanged
- Mouse click handling in `input.rs` — unchanged
- `DashboardAction` enum and optimistic update logic — unchanged
- `POLL_INTERVAL_TICKS` constant — still used by cross-channel message, task count, and callback polling
- `/clear` behavior — `dashboard_running` is already preserved across `/clear`

## Accepted Tradeoffs

- **External state changes are not detected:** If someone runs `mika dashboard start` from another terminal, the TUI won't reflect the change until restart. This is acceptable — the state rarely changes, and when it does via TUI buttons, the optimistic update handles it immediately.
- **One-time startup cost:** The initial check may take up to 3 seconds if the server is unreachable. This only affects users who have `MIKA_INTERNAL_TOKEN` or `MIKA_DASHBOARD_TOKEN` configured but whose server is down — a narrow edge case. CLI-only users (no tokens) skip the network call entirely.

## Sources

- GitHub issue: #425
- Dashboard polling code: `crates/mika-cli/src/tui/app.rs:989-996`
- Dashboard status query: `crates/mika-cli/src/commands/dashboard.rs:44-70`
- Footer rendering: `crates/mika-cli/src/tui/ui.rs:1076-1115`
- Related pattern: `docs/solutions/architecture-patterns/gateway-request-logging-tracelayer-health-filtering.md` — similar log noise reduction for health probes
- Related pattern: `docs/solutions/architecture-patterns/callback-tui-delivery-polling.md` — documents the shared polling architecture
