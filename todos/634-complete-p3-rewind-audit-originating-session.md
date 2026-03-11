---
status: pending
priority: p3
issue_id: "634"
tags: [code-review, observability, rewind]
dependencies: []
---

# Include originating session in cross-session rewind audit event

## Problem Statement

When a cross-session rewind occurs (initiated from session B, deleting in session A), the audit event is logged against session A with no record that session B initiated it. The audit trail is incomplete for forensic analysis.

## Findings

- **Source:** Security sentinel
- **Location:** `crates/mika-agent/src/rewind.rs` lines 423-437 (audit event logging)

## Proposed Solutions

Include the originating session ID in the audit event's `after_value` or `metadata` field when it differs from the target session.

- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] Cross-session rewind audit event includes originating session ID
- [ ] Same-session rewinds unaffected

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | |
