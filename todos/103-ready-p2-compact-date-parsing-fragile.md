---
status: complete
priority: p2
issue_id: "103"
tags: [code-review, quality, robustness]
dependencies: []
---

# Replace string-slicing date parse with chrono in compact_old_memory_events

## Problem Statement
`compact_old_memory_events` extracts year-month from timestamps using string slicing (`&timestamp[..7]`). This assumes the timestamp format is always `YYYY-MM-DD...` and will panic on shorter strings or different formats. Should use `chrono::NaiveDateTime::parse_from_str` for safe parsing.

## Findings
- File: `crates/mika-agent/src/db.rs` (compact_old_memory_events function)
- Uses `&timestamp[..7]` to extract "YYYY-MM" — panics if string < 7 chars
- Timestamps in SQLite are stored as TEXT — format not enforced at DB level
- chrono is already a dependency in the project
- Flagged by: Pattern Recognition Specialist (F-4, Medium), Data Integrity Guardian (Low)

## Proposed Solutions

### Option 1: Parse with chrono (Recommended)
```rust
use chrono::NaiveDateTime;
let dt = NaiveDateTime::parse_from_str(&timestamp, "%Y-%m-%d %H:%M:%S")
    .unwrap_or_else(|_| continue); // skip unparseable rows
let month_key = dt.format("%Y-%m").to_string();
```
**Effort:** Small
**Risk:** Low

### Option 2: Use get() with fallback
```rust
let month_key = timestamp.get(..7).unwrap_or("unknown");
```
**Effort:** Trivial
**Risk:** Low — less correct but avoids panic

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs`

## Acceptance Criteria
- [ ] Date parsing uses chrono or safe string slicing
- [ ] No panic possible on malformed timestamps
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Two agents flagged brittle string-slicing date extraction
