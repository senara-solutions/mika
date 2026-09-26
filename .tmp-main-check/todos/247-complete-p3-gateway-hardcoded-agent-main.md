---
status: complete
priority: p3
issue_id: "247"
tags: [code-review, architecture, future-work]
dependencies: []
---

# Gateway hardcodes `"agent": "main"` -- multi-agent unreachable from Telegram

## Problem Statement

The gateway always sends `"agent": "main"` in forwarded messages. Multi-agent routing from Telegram is non-functional. The server defaults to the active agent when the field is empty, so the hardcoded `"main"` actually overrides the server's default.

## Findings

- **Source:** Code Simplicity, Agent-Native, Architecture, Pattern Recognition
- **File:** `crates/mika-gateway/src/routes.rs:232,313`

## Proposed Solutions

### Option A: Remove the hardcoded field [Recommended for now]
Don't send `"agent"` at all. The server already defaults to the active agent when the field is empty/absent. This is more correct than hardcoding "main".

### Option B: Add `default_agent` column to customers table [Future]
Store per-customer agent preference. Full multi-agent gateway support.

## Acceptance Criteria

- [ ] Gateway either omits the `agent` field or includes a TODO comment

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-25 | Created from PR #12 code review | Multiple agents flagged this |
