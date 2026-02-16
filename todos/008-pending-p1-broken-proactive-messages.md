---
status: pending
priority: p1
issue_id: "008"
tags: [code-review, bug, critical]
dependencies: []
---

# Empty channel_user_id in Proactive Messages (Briefings/Follow-ups Broken)

## Problem Statement

The morning briefing and follow-up Celery tasks query users but do not fetch the associated `UserChannel` to get the `channel_user_id` (e.g., Telegram chat ID). They pass an empty or missing chat ID to the channel adapter's `send_message()`, which means **all proactive messages silently fail or crash**.

**Why it matters:** Core features (morning briefings, follow-up nudges) are completely broken.

## Findings

- **Source:** Python Code Quality Reviewer (#17)
- **Locations:**
  - `app/worker/tasks/briefings.py` — `morning_briefing_dispatcher()`
  - `app/worker/tasks/follow_ups.py` — `follow_up_dispatcher()`
- **Evidence:** Tasks query `User` model but don't join/load `UserChannel` to get `channel_user_id`

## Proposed Solutions

### Option A: Join UserChannel in task queries (Recommended)
- Update dispatcher queries to join `UserChannel` table
- Extract `channel_user_id` for each user's preferred channel
- Pass correct chat ID to `send_message()`
- **Pros:** Simple fix; uses existing data model
- **Cons:** None
- **Effort:** Small
- **Risk:** Low

## Recommended Action
<!-- Filled during triage -->

## Technical Details

**Affected files:**
- `app/worker/tasks/briefings.py`
- `app/worker/tasks/follow_ups.py`
- Possibly `app/models/user.py` (add relationship if missing)

## Acceptance Criteria

- [ ] Morning briefings successfully send to users' Telegram/WhatsApp
- [ ] Follow-up nudges successfully send to users
- [ ] Channel user ID is correctly resolved from UserChannel
- [ ] Tests cover the proactive message flow with correct chat IDs

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | Core feature is non-functional |

## Resources

- Related: `app/models/user.py` UserChannel model
