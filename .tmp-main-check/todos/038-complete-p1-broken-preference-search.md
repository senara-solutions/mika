---
status: complete
priority: p1
issue_id: "038"
tags: [code-review, bug, tools, rust-v2]
dependencies: []
---

# Broken Preference Search in search_memory Tool

## Problem Statement

The `search_memory` tool claims to search preferences by substring but actually does an exact-key HMAC lookup via `db.get_preference(query)`. If a user stored preference `meeting_time: "Morning, before 10am"` and the agent searches for "morning", it finds nothing. This is inconsistent with how people and commitments are searched (which use decrypt-then-filter substring matching).

**Location:** `crates/mika-agent/src/tools/search_memory.rs:110-116`

**Reported by:** pattern-recognition-specialist, agent-native-reviewer, architecture-strategist

## Findings

- Lines 110-116 use `ctx.db.get_preference(query)?` which does HMAC hash lookup against the preference key
- Comment on line 111-112 acknowledges this: "We need to scan all preferences -- no list method exists yet"
- People search (lines 68-88) and commitment search (lines 91-107) both decrypt all records and do substring matching
- Preference search is the only category with degraded search behavior

## Proposed Solutions

### Option A: Add list_preferences() to Database (Recommended)
Add a `list_preferences()` method to `Database` that decrypts all preferences (mirroring `list_people`), then do substring matching in `search_memory`.
- **Pros:** Consistent with other category searches, simple implementation
- **Cons:** O(n) decrypt for all preferences
- **Effort:** Small
- **Risk:** Low

### Option B: Add FTS5 index for preferences
Index preference keys and values in a full-text search table.
- **Pros:** Fast search at scale
- **Cons:** Premature for Phase 1, adds complexity
- **Effort:** Large
- **Risk:** Medium

## Acceptance Criteria

- [ ] `Database` has a `list_preferences()` method that returns all preferences
- [ ] `search_memory` uses substring matching for preferences (consistent with other categories)
- [ ] Existing preference search tests pass
- [ ] New test: search for substring of preference value finds the preference

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | Preference search was known gap per code comment |
| 2026-02-24 | Re-confirmed in encryption-strip review — simpler fix now (plaintext, no HMAC). Need `list_preferences()` in db.rs, then substring match in search_memory.rs | 3 agents flagged: agent-native-reviewer, code-simplicity-reviewer, performance-oracle |
| 2026-02-24 | Fixed — added `list_preferences()` to db.rs, updated search_memory.rs to use substring matching, added test_search_finds_preference_by_value_substring | Verified: searches "shellfish" finds Food preference, searches "meeting" finds Meeting time |
