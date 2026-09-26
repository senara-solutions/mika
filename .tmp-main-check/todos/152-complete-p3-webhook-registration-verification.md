---
status: complete
priority: p3
issue_id: "152"
tags: [plan-review, reliability]
dependencies: []
---

# Add webhook registration verification

## Problem Statement
The plan calls setWebhook on startup but doesn't verify the response or periodically check webhook health. If setWebhook fails silently or the webhook URL changes (e.g., DNS issue), the gateway stops receiving updates with no alerting.

**Why it matters:** Silent webhook deregistration means all customers lose Telegram connectivity with no error signal.

## Findings
- Source: Agent-Native Reviewer (Warning)
- setWebhook can fail if URL is unreachable from Telegram's side
- No periodic getWebhookInfo to verify webhook is healthy
- Telegram provides webhook health stats (pending_update_count, last_error)

## Proposed Solutions

### Option 1: Verify setWebhook response + periodic health check (Recommended)
- Check setWebhook response for `ok: true`
- Log webhook URL on startup for verification
- Optional: periodic getWebhookInfo call (every 5 min) to check pending_update_count and last_error_date
- **Pros**: Early detection of webhook issues
- **Cons**: Extra API calls (minimal)
- **Effort**: Small
- **Risk**: Low

## Acceptance Criteria
- [ ] setWebhook response verified (ok: true)
- [ ] Startup logs include webhook URL (with token redacted)
- [ ] Consider periodic webhook health check

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Agent-Native Reviewer flagged missing webhook verification
