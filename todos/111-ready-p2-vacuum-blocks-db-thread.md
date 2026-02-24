---
status: ready
priority: p2
issue_id: "111"
tags: [code-review, performance, database]
dependencies: []
---

# VACUUM Blocks Entire Database Thread

## Problem Statement

SQLite `VACUUM` rebuilds the entire database file with an exclusive lock. Because `AsyncDatabase` serializes all operations through a single dedicated OS thread, calling VACUUM blocks every queued DB operation for the duration (hundreds of ms to seconds depending on DB size).

In Phase 1 CLI this happens once at startup and is acceptable. In Phase 2 HTTP server, it would stall all concurrent requests during VACUUM.

## Findings

- **Source:** performance-oracle (CRITICAL-2), architecture-strategist
- **Location:** `crates/mika-agent/src/scheduler.rs` lines 41-43; `crates/mika-agent/src/db.rs` lines 1285-1288
- **Evidence:** VACUUM is called inside the async DB thread after compaction. At the 500MB warning threshold, VACUUM could take 2-5 seconds.
- **Note:** The conditional VACUUM (only when `deleted > 0`) from finding #101 was already implemented, which helps. This finding is about replacing VACUUM entirely.

## Proposed Solutions

### Option 1: Switch to incremental auto-vacuum (Recommended)
- **Pros**: Non-blocking, automatic page reclamation, no full file rewrite
- **Cons**: Slightly less space reclamation than full VACUUM
- **Effort**: Small
- **Risk**: Low

Add to database pragmas:
```sql
PRAGMA auto_vacuum = INCREMENTAL;
```
Replace `VACUUM` call with:
```sql
PRAGMA incremental_vacuum(100);  -- reclaim up to 100 pages
```

### Option 2: Run VACUUM on separate connection during idle time
- **Pros**: Zero impact on active operations
- **Cons**: More complex, requires idle detection
- **Effort**: Medium
- **Risk**: Low

## Recommended Action

_To be filled during triage_

## Technical Details

- **Affected Files**: `crates/mika-agent/src/db.rs` (pragmas, vacuum method), `crates/mika-agent/src/scheduler.rs`
- **Database Changes**: PRAGMA change (auto_vacuum must be set before first table creation for new DBs, or requires VACUUM to enable on existing DBs)

## Acceptance Criteria

- [ ] VACUUM no longer called during normal operation
- [ ] Incremental page reclamation works after compaction
- [ ] No stall observed during DB operations when compaction runs
- [ ] Existing tests pass

## Work Log

### 2026-02-24 - Identified in v4 Code Review
**By:** Multi-agent review (performance-oracle, architecture-strategist)
**Actions:** Flagged as P2 performance concern for Phase 2

## Resources

- Commit under review: 38a843b
- SQLite docs: https://www.sqlite.org/pragma.html#pragma_auto_vacuum
