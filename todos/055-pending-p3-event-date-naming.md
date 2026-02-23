---
status: pending
priority: p3
issue_id: "055"
tags: [code-review, naming, tools, rust-v2]
dependencies: []
---

# due_date Parameter Name Incorrect for Events

## Problem Statement
The `store_fact` tool schema uses `due_date` for both commitments and events, but events have an "event date" not a "due date". The DB column is named `event_date` but the tool parameter is `due_date`, creating a semantic mismatch.

**Location:** `crates/mika-agent/src/tools/store_fact.rs:47,192`

**Reported by:** pattern-recognition-specialist

## Proposed Solutions
Rename to `date` in the schema (generic for both) or use `event_date` for events.
- **Effort:** Small

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | |
