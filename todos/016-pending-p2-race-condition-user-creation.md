---
status: pending
priority: p2
issue_id: "016"
tags: [code-review, data-integrity]
dependencies: []
---

# Race Condition in User Creation (Both Channels)

## Problem Statement

Both Telegram and WhatsApp handlers use a "check-then-create" pattern for user creation. Concurrent messages from the same user can create duplicate users.

## Findings

- **Source:** Data Integrity Guardian, Python Code Quality
- **Locations:**
  - `app/channels/telegram/handlers.py` — `_get_or_create_user()`
  - `app/channels/whatsapp/handlers.py` — `_get_or_create_whatsapp_user()`

## Proposed Solutions

### Option A: Use INSERT ON CONFLICT (upsert) (Recommended)
- Replace check-then-create with `INSERT ... ON CONFLICT DO NOTHING/UPDATE`
- Add unique constraint on `UserChannel(channel, channel_user_id)` if not present
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] Concurrent user creation doesn't produce duplicates
- [ ] Unique constraint enforced at database level
- [ ] Existing tests still pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
