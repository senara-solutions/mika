---
status: pending
priority: p3
issue_id: "023"
tags: [code-review, security]
dependencies: []
---

# LIKE Injection in Memory Search

## Problem Statement

Memory search endpoints pass user input directly into SQL LIKE patterns without escaping `%` and `_` wildcard characters. Users can craft inputs to match unintended patterns.

## Findings

- **Source:** Security Sentinel (M2)
- **Location:** Memory search/query functions

## Proposed Solutions

### Option A: Escape LIKE wildcards (Recommended)
- Escape `%`, `_`, and `\` in user input before LIKE queries
- **Effort:** Small | **Risk:** Low

## Acceptance Criteria

- [ ] User input is escaped before LIKE queries
- [ ] Wildcard characters in input don't affect query behavior

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-16 | Created from code review | |
