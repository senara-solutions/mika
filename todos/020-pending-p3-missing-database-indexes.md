---
status: pending
priority: p3
issue_id: "020"
tags: [code-review, performance, database]
dependencies: []
---

# Missing Database Indexes

## Problem Statement

Several frequently queried columns lack indexes, which will degrade performance as data grows.

## Findings

- **Source:** Performance Oracle
- **Missing indexes:**
  - `user_channels.channel + channel_user_id` (composite, used in every message)
  - `conversations.user_id` (filtered in queries)
  - `messages.conversation_id` (JOIN target)
  - `user_consents.user_id` (filtered)

## Proposed Solutions

### Option A: Add indexes via migration (Recommended)
- Create Alembic migration adding the missing indexes
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] All identified indexes are created
- [ ] Migration runs successfully
- [ ] Query plans show index usage

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
