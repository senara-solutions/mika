---
status: complete
priority: p2
issue_id: "444"
tags: [code-review, data-integrity]
dependencies: []
---

# Dual Timestamp Representation — TeamRun vs DB

## Problem Statement

`TeamRun.started_at` is `String` (RFC 3339) while the DB stores `i64` (Unix timestamp). Separate `chrono::Utc::now()` calls at lines 115 and 137 can diverge. Same bug class that v9 migration fixed for reminders.

## Fix

Change `TeamRun.started_at` and `ended_at` to `i64` / `Option<i64>`, use `chrono::Utc::now().timestamp()` everywhere, format only at display time.

## Acceptance Criteria

- [ ] `TeamRun` uses `i64` timestamps
- [ ] Single `Utc::now().timestamp()` call per timestamp
- [ ] Tests updated
