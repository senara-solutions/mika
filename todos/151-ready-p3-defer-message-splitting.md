---
status: ready
priority: p3
issue_id: "151"
tags: [plan-review, simplicity]
dependencies: []
---

# Defer message splitting — YAGNI until proven needed

## Problem Statement
The plan includes a message splitting algorithm (split at 4096 char Telegram limit, respect paragraph/sentence boundaries). This adds 20-30 lines of boundary-detection logic. In practice, Claude responses forwarded through the container are already sized by claude_max_tokens (4096 tokens ≈ ~3000 chars for English), likely under the Telegram limit.

**Why it matters:** Premature optimization for a scenario that may not occur. If it does occur, Telegram will simply truncate — not ideal but not catastrophic.

## Findings
- Source: Code Simplicity Reviewer (YAGNI)
- Claude max_tokens default is 4096 tokens ≈ ~3000 chars (under 4096 char limit)
- Message splitting with boundary detection is complex to get right
- Can be added later if truncation is observed in practice

## Proposed Solutions

### Option 1: Send as-is, add splitting only if needed (Recommended)
Send the full text to Telegram. If it's over 4096 chars, Telegram will reject it — catch the error, log it, and add splitting then.
- **Pros**: Zero complexity now, data-driven decision later
- **Cons**: Rare edge case of truncated messages until splitting is added
- **Effort**: None now
- **Risk**: Low

### Option 2: Simple truncation at 4096 chars
If text > 4096, truncate and append "..." — no boundary detection.
- **Pros**: Simple one-liner fallback
- **Cons**: May cut mid-word
- **Effort**: Small
- **Risk**: Low

## Acceptance Criteria
- [ ] No message splitting in initial implementation
- [ ] Telegram API errors for oversized messages are logged with message length
- [ ] Splitting added later if truncation is observed in production

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent plan review)
**Actions:** Code Simplicity Reviewer flagged message splitting as premature optimization
