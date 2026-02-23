---
status: pending
priority: p3
issue_id: "054"
tags: [code-review, quality, database, rust-v2]
dependencies: []
---

# Decrypt-or-Skip Pattern Duplicated ~150 Lines in db.rs

## Problem Statement
The two-stage filter_map pattern (1. unwrap rusqlite::Result, 2. decrypt fields) is repeated 5 times in db.rs across `load_recent_messages`, `get_all_core_memory`, `list_people`, `list_commitments`, and `get_memory_events`.

**Location:** `crates/mika-agent/src/db.rs` (multiple methods)

**Reported by:** pattern-recognition-specialist

## Proposed Solutions
Extract a `decrypt_optional` helper method or a generic row-processing closure pattern.
- **Effort:** Medium (careful refactoring needed for different field structures)

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | ~150 lines of structural duplication |
