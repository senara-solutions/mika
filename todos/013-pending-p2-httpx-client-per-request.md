---
status: pending
priority: p2
issue_id: "013"
tags: [code-review, performance, whatsapp]
dependencies: []
---

# httpx Client Created Per Request in WhatsApp Adapter

## Problem Statement

`WhatsAppAdapter.send_message()` creates a new `httpx.AsyncClient()` for every message sent. This wastes TCP connections, skips connection pooling, and adds latency.

## Findings

- **Source:** Performance Oracle (CRITICAL-1), Pattern Recognition
- **Location:** `app/channels/whatsapp/__init__.py` — `async with httpx.AsyncClient() as client:`

## Proposed Solutions

### Option A: Use a shared client instance (Recommended)
- Create `httpx.AsyncClient` once in `__init__()` with connection pooling
- Add proper cleanup in `close()` method
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] Single httpx.AsyncClient instance shared across requests
- [ ] Connection pooling is active
- [ ] Client is properly closed on shutdown

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
