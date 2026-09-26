# Plan Callout Recognition: Branch + Plan Contract

## Ticket: mika#1225 — Implement session timeout handling

### Description

Sessions that have been idle for more than 30 minutes should be automatically
closed with a system message indicating the timeout. This prevents resource leaks
from abandoned conversations.

### Issue Body

The timeout scanner runs as a background task on the engine tick (every 60s).
It queries `sessions` for `updated_at < now() - 30min` where `status = 'active'`.

For each timed-out session:
1. Insert a system message: "Session closed due to inactivity (30 min timeout)"
2. Update session status to `closed`
3. Emit a `session_timeout` structured log event

> - **Branch:** `feat/1225/session-timeout-handling`
> - **Plan:** `docs/plans/1225-session-timeout.md`

### Acceptance Criteria

- [ ] Idle sessions are closed after 30 minutes
- [ ] System message is inserted before close
- [ ] Timeout duration is configurable via `MIKA_SESSION_TIMEOUT_SECS`
- [ ] Unit test with mock clock verifies timeout detection
- [ ] No false positives on sessions with recent tool calls (check messages table too)
