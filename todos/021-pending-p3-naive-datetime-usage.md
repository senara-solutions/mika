---
status: pending
priority: p3
issue_id: "021"
tags: [code-review, quality, python]
dependencies: []
---

# Naive datetime.now() Without Timezone

## Problem Statement

Multiple locations use `datetime.now()` or `datetime.utcnow()` which return naive datetimes. This can cause timezone-related bugs, especially with users in different timezones.

## Findings

- **Source:** Python Code Quality Reviewer
- **Pattern:** `datetime.now()` and `datetime.utcnow()` should be `datetime.now(UTC)` or `datetime.now(tz=timezone.utc)`

## Proposed Solutions

### Option A: Replace with timezone-aware datetimes (Recommended)
- Use `datetime.now(timezone.utc)` everywhere
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] No naive `datetime.now()` or `datetime.utcnow()` in codebase
- [ ] All datetimes are timezone-aware

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
