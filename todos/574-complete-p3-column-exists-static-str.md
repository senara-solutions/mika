---
status: complete
priority: p3
issue_id: "574"
tags: [code-review, security, defensive-coding]
dependencies: []
---

# Harden column_exists() with static string types

## Problem Statement

The `column_exists()` helper interpolates `table` into SQL via `format!()`. While all current call sites are hardcoded literals, the function signature accepts `&str` which could allow dynamic input from a future caller.

## Findings

- **Source:** Security Sentinel, Architecture Strategist, Data Integrity Guardian (all flagged independently)
- **File:** `crates/mika-agent/src/db.rs:883`
- **Current risk:** None (hardcoded callers only)
- **Future risk:** Low — but easy to harden

## Proposed Solutions

### Option A: Use &'static str parameters (Recommended)
Change signature to `fn column_exists(&self, table: &'static str, column: &'static str)`. Enforces compile-time literal strings.

- **Effort:** Small (5 min)
- **Risk:** None

## Acceptance Criteria

- [ ] `column_exists` only accepts compile-time string literals

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from PR #88 code review | Defensive hardening for SQL formatting |
