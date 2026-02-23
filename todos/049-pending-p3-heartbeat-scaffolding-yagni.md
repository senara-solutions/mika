---
status: pending
priority: p3
issue_id: "049"
tags: [code-review, yagni, rust-v2]
dependencies: []
---

# Heartbeat Scaffolding Is YAGNI

## Problem Statement
The entire heartbeat feature is scaffolded (migration v3 creates `heartbeat_log` table, `log_heartbeat()` DB method, `heartbeat.md` default file) but nothing calls any of it. This is dead code added for a feature that doesn't exist yet.

**Location:** db.rs (migrate_v3, log_heartbeat), home.rs (DEFAULT_HEARTBEAT)

**Reported by:** code-simplicity-reviewer

## Proposed Solutions
Remove heartbeat scaffolding entirely and add it back when the feature is built. Or keep it if Phase 2 is imminent.
- **Effort:** Small

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | ~50 LOC including tests |
