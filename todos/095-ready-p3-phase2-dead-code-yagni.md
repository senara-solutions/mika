---
status: ready
priority: p3
issue_id: "095"
tags: [code-review, quality, yagni]
dependencies: []
---

# Remove Phase 2 dead code with zero production callers

## Problem Statement
Several methods, structs, and an entire module exist with zero production callers — written for Phase 2 features that don't exist yet. This adds ~880 lines of maintenance burden. Schema migrations should stay (avoid future migration churn), but Rust methods should be written when their callers are built.

## Findings
- `async_db.rs` (729 lines): 46 wrapper methods, zero production callers. All code paths use `&Database` directly.
- `failed_sends` methods (4) + `FailedSend` struct: `record_failed_send`, `get_failed_sends`, `delete_failed_send`, `increment_failed_send_retry` — zero callers
- Heartbeat methods (3): `record_heartbeat_send`, `count_heartbeat_sends_today`, `count_heartbeat_sends_last_hour` — zero callers (only `prune_old_heartbeat_sends` is called)
- `last_user_message_time`: zero callers
- `save_conversation_summary` and `delete_compacted_messages` are `pub` but only called internally by `replace_with_summary`
- Flagged by: Code Simplicity Reviewer

## Proposed Solutions

### Option 1: Conservative cleanup (Recommended)
- Remove `async_db.rs` entirely — rewrite with `with_db` helper in Phase 2 PR
- Remove 4 `failed_sends` methods + `FailedSend` struct (keep table migration)
- Remove 3 unused heartbeat methods (keep `prune_old_heartbeat_sends`)
- Remove `last_user_message_time`
- Change `save_conversation_summary` and `delete_compacted_messages` to private
**Effort:** Small
**Risk:** Low (no production code depends on any of this)

### Option 2: Keep async_db, remove only dead methods
Keep `async_db.rs` as Phase 2 scaffolding but remove the smaller dead methods.
**Effort:** Trivial
**Risk:** 729 lines of maintenance burden remains

## Technical Details
**Affected files:** `crates/mika-agent/src/async_db.rs`, `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/lib.rs`

## Acceptance Criteria
- [ ] No methods with zero production callers remain (except schema migrations)
- [ ] `save_conversation_summary` and `delete_compacted_messages` are private
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified ~880 lines of dead code with zero production callers
