---
status: ready
priority: p2
issue_id: "140"
tags: [plan-review, architecture, simplicity]
dependencies: []
---

# Simplify or remove in-memory state (dedup + bot blocked tracking)

## Problem Statement
The plan includes two in-memory tracking systems: (1) update_id dedup via HashMap, and (2) bot blocked detection via consecutive 403 counter per chat_id. Both are lost on restart, broken for multi-replica, and add complexity. The Code Simplicity Reviewer flagged both as YAGNI — Telegram already guarantees at-least-once delivery with update_id ordering, and the bot blocked auto-suspension creates a mass-suspension attack vector.

**Why it matters:** In-memory state that is lost on restart gives a false sense of protection. Multi-replica deployments break the guarantee entirely. The auto-suspension feature is actively dangerous (attacker blocks bot from multiple accounts → mass suspension).

## Findings
- Source: Code Simplicity Reviewer (YAGNI), Performance Oracle (Critical), Architecture Strategist, Security Sentinel (H-3)
- 4 out of 6 review agents flagged in-memory state as problematic
- Dedup: Telegram's update_id is monotonically increasing — can use simple last_processed_id check
- Bot blocked: Auto-suspension is exploitable — attacker uses multiple accounts to block bot, triggering auto-suspend for legitimate customers
- Both features add ~50 lines of code for questionable value

## Proposed Solutions

### Option 1: Remove both, use simple last_update_id in Postgres (Recommended)
- Store `last_update_id BIGINT` per customer in Postgres
- Skip updates with id ≤ last_update_id (simple, persistent, multi-replica safe)
- Remove bot blocked detection entirely (YAGNI — handle 403s as transient errors, log for manual review)
- **Pros**: Simpler, persistent, multi-replica safe, removes attack vector
- **Cons**: One extra Postgres write per message (negligible)
- **Effort**: Small (net code reduction)
- **Risk**: Low

### Option 2: Persist both to Postgres
- Move dedup HashMap to `processed_updates` table
- Move 403 counter to `blocked_status` table
- **Pros**: Persistent, multi-replica safe
- **Cons**: Still has auto-suspension attack vector, more complex than needed
- **Effort**: Medium
- **Risk**: Medium (auto-suspension risk remains)

## Technical Details
- **Affected files**: Plan Phase 3.3 (routing.rs), schema
- **Related Components**: Webhook processing, container forwarding

## Acceptance Criteria
- [ ] No in-memory state that is lost on restart
- [ ] Dedup is persistent and multi-replica safe
- [ ] No auto-suspension attack vector
- [ ] All 403 errors logged for manual investigation

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** 4 agents converged on removing/simplifying in-memory state
