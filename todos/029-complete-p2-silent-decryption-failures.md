---
status: complete
priority: p2
issue_id: "029"
tags: [code-review, security, data-integrity, rust-v2]
dependencies: []
---

# Decryption Failures Silently Dropped via filter_map

## Problem Statement

Multiple database query methods use `.filter_map(|r| r.ok())` to silently skip rows where decryption fails. If the encryption key is wrong, rotated, or data is corrupted, the agent loads an incomplete conversation history and empty memory without any error.

**Why it matters:** For an executive assistant selling persistent personalization, silent data loss is catastrophic. The agent would behave as a fresh install, forgetting everything.

## Findings

- **Source:** Security Sentinel (M4), Architecture Strategist (H, R2), Performance Oracle (OPT-4)
- **Locations:** `crates/mika-agent/src/db.rs` lines 222-233, 288-289, 428-431, 495-496
- **Pattern:** `.filter_map(|r| r.ok()).filter_map(|raw| { self.key.decrypt_string(...).ok()? })`

## Proposed Solutions

### Option A: Log warnings + fail on startup key check (Recommended)
- Add `tracing::warn!` when decryption fails (with row ID, no plaintext)
- Add `db.check_encryption_key()` on startup that decrypts a known row, failing readiness if wrong
- Keep filter_map for runtime resilience but make failures visible
- **Pros:** Loud failures, graceful degradation for single corrupt rows
- **Cons:** Slightly more complex code
- **Effort:** Small
- **Risk:** Low

### Option B: Fail entire operation on any decryption error
- Replace filter_map with collect::<Result<Vec<_>>>()
- **Pros:** Strictest — no silent data loss
- **Cons:** Single corrupt row breaks everything
- **Effort:** Small
- **Risk:** Medium (one bad row takes down the agent)

## Acceptance Criteria

- [ ] Decryption failures logged at WARN level with row identifier
- [ ] Startup key validation fails the readiness probe if key is wrong
- [ ] Agent does not silently lose conversation history
