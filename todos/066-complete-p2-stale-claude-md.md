---
status: complete
priority: p2
issue_id: "066"
tags: [code-review, documentation, rust-v2]
dependencies: []
---

# CLAUDE.md Contains Stale Encryption References and Outdated Counts

## Problem Statement

The project-level `CLAUDE.md` still references encryption concepts that were removed in prior commits: "encrypted SQLite storage", "AES-256-GCM", "HMAC-SHA256 for lookups", "`EncryptionKey` uses `ZeroizeOnDrop`", "`MIKA_ENCRYPTION_KEY`". It also says "32 tests" when there are now 71, and describes `ToolContext` as `{ db }` when it has more fields.

**Why it matters:** CLAUDE.md is the primary onboarding document for AI assistants and contributors. Stale instructions cause confusion and incorrect assumptions.

## Findings

- **Source:** architecture-strategist, security-sentinel
- **Location:** `CLAUDE.md` — Stack, Conventions, Environment Variables, Architecture sections
- **Evidence:** Multiple references to encryption that no longer exists in the codebase

## Proposed Solutions

### Option A: Update CLAUDE.md to match current state (Recommended)
- Remove all encryption references (AES-256-GCM, HMAC, EncryptionKey, MIKA_ENCRYPTION_KEY)
- Update test count to 71
- Update ToolContext description
- Update tool descriptions to include update_fact
- Add note about K8s volume encryption as the security boundary
- **Pros:** Accurate documentation
- **Cons:** None
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] No encryption references remain in CLAUDE.md (except K8s volume mention)
- [ ] Test count updated
- [ ] ToolContext description accurate
- [ ] Tool list includes update_fact
- [ ] Environment variables section updated (no MIKA_ENCRYPTION_KEY)

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review of commit 3619d13 | CLAUDE.md must be updated whenever major refactors ship |
| 2026-02-24 | Resolved: updated test count (127->130), added update_fact/store_fact/search_memory to Layer 2 description. Encryption refs, ToolContext, env vars, and platform systems were already accurate from prior fixes. | Most items in original todo were already resolved by earlier commits; always re-check current state before editing |
