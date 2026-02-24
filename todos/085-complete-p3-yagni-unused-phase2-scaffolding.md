---
status: pending
priority: p3
issue_id: "085"
tags: [code-review, quality, yagni]
dependencies: []
---

# Consider removing unused Phase 2 scaffolding code

## Problem Statement
~1,010 lines (~22% of the +4,642 added) are Phase 2 scaffolding with zero production callers. This code must be maintained through every API change until Phase 2, adding review burden and confusion about what the system actually does.

## Findings
1. **async_db.rs** (736 lines) — entire file unused. No import outside own tests. AsyncDatabase wrapper built but nothing uses it.
2. **failed_sends** table + 4 methods + struct + tests (~80 lines) — retry queue for HTTP gateway that doesn't exist
3. **heartbeat_sends** 4 rate-limiting methods + tests (~75 lines) — heartbeat timer doesn't exist, SilentTrigger::Heartbeat never constructed
4. **customer_config** 2 methods + tests (~40 lines) — per-customer settings for non-existent gateway
5. **last_user_message_time** method + test (~25 lines) — staleness check for non-existent heartbeat
6. **CliMessageSender** (~8 lines) — never instantiated, None fallback handles CLI mode
7. **SilentTrigger::Heartbeat** match arms (~15 lines) — unreachable code paths

**Counterargument:** async_db.rs was explicitly planned as Phase 0 infrastructure. The DDL tables in migration v5 are pre-production (no data to preserve). Some teams prefer having the scaffolding ready.

## Proposed Solutions
### Option 1: Remove all unused code (Aggressive)
Delete async_db.rs, unused DB methods/structs/tests, CliMessageSender, Heartbeat variant. Optionally remove unused table DDL from migration.
**Effort:** 1 hour | **Risk:** Low (no production callers)

### Option 2: Keep async_db.rs, remove only truly dead code (Conservative)
Keep async_db.rs as planned Phase 0 deliverable. Remove: failed_sends methods, heartbeat_sends methods, customer_config methods, last_user_message_time, CliMessageSender.
**Effort:** 30 minutes | **Risk:** Low

### Option 3: Keep everything, add #[allow(dead_code)] annotations
Document intent with comments, suppress warnings.
**Effort:** 15 minutes | **Risk:** Low but maintains debt

## Recommended Action
Team decision — depends on preference for scaffolding vs. YAGNI.

## Technical Details
**Affected files:**
- `crates/mika-agent/src/async_db.rs` — entire file
- `crates/mika-agent/src/db.rs` — multiple methods/structs
- `crates/mika-agent/src/messaging.rs` — CliMessageSender
- `crates/mika-agent/src/agent.rs` — SilentTrigger::Heartbeat
- `crates/mika-agent/src/lib.rs` — async_db module declaration

## Acceptance Criteria
- [ ] Decision documented
- [ ] Chosen code removed or annotated
- [ ] Tests pass, clippy clean

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review)
**Actions:** Identified ~1,010 lines of unused Phase 2 scaffolding
