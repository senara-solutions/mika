---
status: pending
priority: p2
issue_id: "327"
tags: [code-review, security, performance]
dependencies: []
---

# File Download Completes Before Size Check

## Problem Statement

`download_file_bytes()` in `crates/mika-gateway/src/telegram.rs` downloads the entire file into memory before `download_image()` checks the 5MB size limit. Telegram Bot API allows files up to 20MB, so a file between 5-20MB is fully buffered then discarded.

## Findings

- Flagged by: security-sentinel (Medium), performance-oracle
- Location: `crates/mika-gateway/src/telegram.rs:356-390`
- With 30 semaphore permits, worst case is 30 * 20MB = 600MB before rejection

## Proposed Solutions

### Option A: Check Content-Length header + streaming abort
- **Pros:** Early rejection via Content-Length, streaming caps memory
- **Cons:** Requires `futures-util` for `StreamExt`
- **Effort:** Medium
- **Risk:** Low

### Option B: Just check Content-Length header
- **Pros:** Simple, no new dependencies
- **Cons:** Content-Length can be missing or lying
- **Effort:** Small
- **Risk:** Low (partial protection)

## Acceptance Criteria

- [ ] Files >5MB rejected without full download into memory
- [ ] Content-Length checked before streaming if available
