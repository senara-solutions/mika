---
status: pending
priority: p2
issue_id: "018"
tags: [code-review, architecture, quality]
dependencies: []
---

# Duplicate Code Across Channel Handlers

## Problem Statement

Telegram and WhatsApp handlers duplicate user creation logic, message processing flow, and agent invocation patterns. Changes to one handler must be mirrored in the other.

## Findings

- **Source:** Pattern Recognition, Code Simplicity
- **Locations:**
  - `app/channels/telegram/handlers.py`
  - `app/channels/whatsapp/handlers.py`
- **Evidence:** `_get_or_create_user()` vs `_get_or_create_whatsapp_user()` are ~80% identical

## Proposed Solutions

### Option A: Extract shared service layer (Recommended)
- Create `app/channels/services.py` with `get_or_create_user(channel, channel_user_id, name)`
- Create `process_message(user, text, channel)` shared function
- Handlers become thin wrappers that parse channel-specific formats
- **Effort:** Medium | **Risk:** Low

## Acceptance Criteria

- [ ] User creation logic is in one place
- [ ] Message processing logic is shared
- [ ] Both channel handlers work correctly
- [ ] Tests cover the shared service layer

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
