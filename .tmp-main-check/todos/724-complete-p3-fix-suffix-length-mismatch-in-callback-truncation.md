---
status: pending
priority: p3
issue_id: 724
tags: [code-review, quality]
dependencies: []
---

# Fix suffix length mismatch in callback result truncation

## Problem Statement

In `format_callback_framing()`, the truncation logic subtracts 3 from the max byte count (matching `truncate_summary()`'s "..." suffix) but appends a 55-byte suffix. This means the total truncated output is ~10,292 bytes instead of the intended ~10,240. At 10KB scale this is negligible, but it's incorrect by construction.

## Findings

- `let cut = CALLBACK_RESULT_MAX_BYTES.saturating_sub(3);` — reserves 3 bytes for suffix
- Actual suffix: `"\n...\n[truncated — full result available in task logs]"` — ~55 bytes
- Net effect: content is ~52 bytes over budget (10,237 + 55 = 10,292 vs target 10,240)
- The simplicity reviewer suggests extracting a shared `truncate_at_boundary(s, max_len, suffix)` helper that uses `suffix.len()` for the subtraction

## Proposed Solutions

### Solution 1: Use suffix.len() inline (Simple)
```rust
let suffix = "\n...\n[truncated — full result available in task logs]";
let cut = CALLBACK_RESULT_MAX_BYTES.saturating_sub(suffix.len());
```
- **Effort**: Small
- **Risk**: None

### Solution 2: Extract shared helper (Broader, deferred)
Extract `truncate_at_boundary(s, max_len, suffix)` and reuse from `truncate_summary()`. Better long-term but larger change.
- **Effort**: Medium
- **Risk**: Low

## Technical Details

- **Affected files**: `crates/mika-agent/src/agent.rs`

## Acceptance Criteria

- [ ] Suffix length subtraction uses actual suffix length, not hardcoded 3
