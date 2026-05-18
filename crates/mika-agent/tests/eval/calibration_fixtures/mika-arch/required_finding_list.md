# Required Finding List: Plan With Deliberate Issues (ESCALATE)

## Plan Under Review: docs/plans/1245-webhook-retry-queue.md

### Summary

Add a retry queue for failed webhook deliveries so transient downstream failures
don't cause permanent message loss.

### Design

- Store failed webhooks in a new `webhook_retry_queue` SQLite table
- Background task polls every 5 seconds and retries pending deliveries
- After 3 failures, mark as permanently failed

### Implementation

Add the table, add the background task, wire it up in server startup.
Should be straightforward — just follow the pattern from the task engine.

### Error Handling

(not specified)

### Test Plan

(not specified)

### Scope

The retry queue handles all webhook types (Telegram, GitHub, A2A, future channels).
It should also handle rate limiting, circuit breaking, priority queuing, dead letter
archival, and webhook signature rotation.

### Notes

- We might need to change the database schema
- Probably needs some kind of backoff
- Not sure if this should be per-agent or global
