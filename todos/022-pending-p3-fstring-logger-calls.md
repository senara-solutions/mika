---
status: pending
priority: p3
issue_id: "022"
tags: [code-review, quality, python]
dependencies: []
---

# f-strings in Logger Calls (Deferred Formatting)

## Problem Statement

Logger calls use f-strings (`logger.info(f"Processing {x}")`) instead of lazy formatting (`logger.info("Processing %s", x)`). This evaluates the string even when the log level is disabled, wasting CPU.

## Findings

- **Source:** Python Code Quality Reviewer
- **Pattern:** Found throughout codebase

## Proposed Solutions

### Option A: Use % formatting in logger calls (Recommended)
- Replace `logger.info(f"msg {var}")` with `logger.info("msg %s", var)`
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] No f-strings in logger.debug/info/warning/error calls
- [ ] All tests still pass

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
