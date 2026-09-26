---
status: complete
priority: p2
issue_id: "058"
tags: [code-review, documentation, rust-v2]
dependencies: []
---

# Stale Encryption References in README.md

## Problem Statement

`README.md` still contains multiple references to field-level encryption that no longer exists: AES-256-GCM, HMAC-SHA256, EncryptionKey, MIKA_ENCRYPTION_KEY env var, and "encrypted at rest" descriptions.

**Location:** `README.md` — multiple lines

**Reported by:** code-simplicity-reviewer

## Findings

- README.md was not listed in the encryption strip plan and was missed
- CLAUDE.md was updated but README.md was not
- References exist in stack description, conventions, environment variables, and architecture sections

## Proposed Solutions

### Option A: Update README.md to match CLAUDE.md (Recommended)
Mirror the same changes applied to CLAUDE.md: remove encryption mentions, update stack description, remove MIKA_ENCRYPTION_KEY from env vars.
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria

- [ ] No references to AES-256-GCM, HMAC-SHA256, EncryptionKey, or MIKA_ENCRYPTION_KEY in README.md
- [ ] Stack and architecture sections reflect plaintext + volume-level encryption model

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from encryption-strip code review | README.md was not in the plan's file list |
