---
status: pending
priority: p3
issue_id: "107"
tags: [code-review, documentation]
dependencies: []
---

# Update CLAUDE.md to remove async_db references

## Problem Statement
CLAUDE.md still references `async_db.rs` and `AsyncDatabase` in the Conventions section and Pending Work, but `async_db.rs` was deleted in this PR. The documentation is now stale.

## Findings
- File: `CLAUDE.md`
- Conventions section mentions: "AsyncDatabase wraps sync Database with Arc<Mutex<Database>> + tokio::task::spawn_blocking"
- Pending Work mentions: "todo #027 — async_db.rs created"
- `async_db.rs` was removed as dead code in this PR (todo #095)
- Flagged by: Architecture Strategist

## Proposed Solutions

### Option 1: Remove async_db references, update Pending Work (Recommended)
- Remove the `Async DB` convention bullet
- Update Pending Work to say async wrapper will be recreated in Phase 2
**Effort:** Trivial
**Risk:** None

## Technical Details
**Affected files:** `CLAUDE.md`

## Acceptance Criteria
- [ ] No references to async_db.rs in CLAUDE.md
- [ ] Pending Work accurately reflects Phase 2 plans

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Architecture Strategist identified stale documentation
