---
status: wont_fix
priority: p3
issue_id: "109"
tags: [code-review, architecture]
dependencies: []
---

# Split db.rs into submodules when adding Phase 2 features

## Problem Statement
`db.rs` has grown to 1953 lines and contains schema migrations, all CRUD operations, types, search methods, and tiered retention. When Phase 2 adds more features, this file will become unwieldy. Plan to split into submodules.

## Findings
- File: `crates/mika-agent/src/db.rs` (1953 lines)
- Contains: schema (285 lines), types/structs, CRUD for 7+ tables, search methods, compaction, vacuum
- Natural split points: schema/migrations, types, per-table CRUD, compaction
- Not urgent for Phase 1 — file is still navigable
- Flagged by: Architecture Strategist, Pattern Recognition Specialist

## Proposed Solutions

### Option 1: Split during Phase 2 (Recommended)
When adding async wrapper and new features, split into:
```
db/
  mod.rs          -- Database struct, new(), migration runner
  schema.rs       -- SQL migrations
  types.rs        -- Structs (Commitment, Preference, Event, etc.)
  memory.rs       -- Core memory, facts, search
  reminders.rs    -- Reminder CRUD
  compaction.rs   -- Tiered retention, vacuum
  events.rs       -- Memory events, audit log
```
**Effort:** Medium (but natural during Phase 2 refactor)
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/db.rs` → `crates/mika-agent/src/db/`

## Acceptance Criteria
- [ ] db.rs split into logical submodules during Phase 2
- [ ] Public API unchanged
- [ ] All tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v3 - PR #4)
**Actions:** Two agents flagged db.rs size as growing concern
