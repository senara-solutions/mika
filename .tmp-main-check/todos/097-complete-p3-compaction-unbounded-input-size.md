---
status: complete
priority: p3
issue_id: "097"
tags: [code-review, security, performance]
dependencies: []
---

# Add total character budget for compaction summarization input

## Problem Statement
The compaction `summarize_messages()` function caps the batch at 100 messages and output at 4000 chars, but there is no cap on total input size. With `MAX_INPUT_LEN` of 10,000 chars per message, 100 messages could send up to 1,000,000 characters to the Claude API for summarization, causing API cost amplification.

## Findings
- File: `crates/mika-agent/src/compaction.rs:81-121`
- Batch capped at `MAX_COMPACTION_BATCH = 100` messages
- Output capped at `MAX_SUMMARY_CHARS = 4000`
- No input budget — total input chars uncapped
- Could amplify API costs if attacker generates many large messages
- Flagged by: Security Sentinel (Informational)

## Proposed Solutions

### Option 1: Add input character budget (Recommended)
```rust
const MAX_COMPACTION_INPUT_CHARS: usize = 50_000;
// In summarize_messages:
let mut char_count = 0;
let truncated: Vec<_> = messages.iter().take_while(|m| {
    char_count += m.content.len();
    char_count <= MAX_COMPACTION_INPUT_CHARS
}).collect();
```
**Effort:** Small
**Risk:** Low

## Technical Details
**Affected files:** `crates/mika-agent/src/compaction.rs`

## Acceptance Criteria
- [ ] Total input chars for summarization capped
- [ ] Tests pass

## Work Log
### 2026-02-24 - Discovery
**By:** Claude Code (multi-agent review v2)
**Actions:** Identified unbounded summarization input size
