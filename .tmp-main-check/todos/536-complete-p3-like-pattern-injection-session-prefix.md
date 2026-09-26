---
status: complete
priority: p3
issue_id: 536
tags: [code-review, security, database]
dependencies: []
---

# LIKE Pattern Metacharacters Not Escaped in Session Prefix Query

## Problem Statement

`count_pending_callback_tasks_by_session_prefix` builds a LIKE pattern from `session_prefix` without escaping `%` and `_` metacharacters. Currently the prefix is UUID-derived so not exploitable, but a future caller passing user-controlled input could cause incorrect results.

**Severity:** P3 — Not exploitable today, defensive hardening.

## Findings

- `crates/mika-agent/src/db.rs:1011-1023` — `format!("{session_prefix}%")` without escaping

## Proposed Solutions

1. **Escape LIKE metacharacters and add ESCAPE clause**
   - Effort: Small
   - Risk: Low

2. **Replace LIKE query with team_run_id filter** (see also #532)
   - `WHERE team_run_id = ?1 AND trigger_type = 'callback' AND status = 'pending' AND depth > 1`
   - Pros: Simpler, index-friendly, no LIKE needed
   - Effort: Small
   - Risk: Low

## Acceptance Criteria

- [ ] LIKE metacharacters escaped, or LIKE query replaced with team_run_id filter
