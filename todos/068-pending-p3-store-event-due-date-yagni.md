---
status: pending
priority: p3
issue_id: "068"
tags: [code-review, quality, rust-v2]
dependencies: []
---

# due_date Backward Compatibility Fallback in store_event is YAGNI

## Problem Statement

`store_event` accepts `event_date` with a fallback to `due_date`. The tool schema already declares `event_date` as the field name. The LLM is the only caller and reads the schema. There are no external API callers requiring backward compatibility. The fallback adds 4 lines of code and a 22-line test for a transition that never existed.

## Findings

- **Source:** code-simplicity-reviewer
- **Location:** `crates/mika-agent/src/tools/store_fact.rs:196-199` (fallback), lines 322-343 (test)

## Proposed Solutions

### Option A: Remove due_date fallback (Recommended)
- Only accept `event_date`
- Remove `test_store_event_due_date_fallback` test
- **Pros:** ~26 lines removed, no unnecessary compatibility shim
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] `store_event` only reads `event_date`, no `due_date` fallback
- [ ] Fallback test removed
- [ ] All other tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review of commit 3619d13 | LLM-only tools don't need backward compat for field renames |
