---
status: pending
priority: p3
issue_id: 723
tags: [code-review, testing]
dependencies: []
---

# Improve UTF-8 boundary walk-back test coverage in callback truncation

## Problem Statement

The `test_format_callback_framing_truncation_utf8_safe` test places a 4-byte emoji at `CALLBACK_RESULT_MAX_BYTES - 2` which puts the emoji AFTER the cut point (cut = max - 3 = 10_237, emoji starts at 10_238). The char-boundary walk-back is never exercised because the cut lands on a valid 'a' boundary.

## Findings

- Current test: `"a" × 10_238 + 🦀 + "zzz"` — cut at 10_237 is already a valid char boundary (last 'a')
- To exercise the walk-back, the emoji should straddle the cut point: `"a" × 10_236 + 🦀 + "zzz"` — cut at 10_237 lands inside the 4-byte emoji (bytes 10_236-10_239), forcing boundary to walk back to 10_236

## Proposed Solutions

### Solution 1: Fix the emoji offset (Recommended)
Change `CALLBACK_RESULT_MAX_BYTES - 2` to `CALLBACK_RESULT_MAX_BYTES - 4` so the 4-byte emoji starts exactly at the cut point:
```rust
let mut s = "a".repeat(CALLBACK_RESULT_MAX_BYTES - 4);
s.push('🦀'); // 4-byte char, starts at cut-1, straddles the boundary
```
- **Effort**: Small
- **Risk**: None

## Technical Details

- **Affected files**: `crates/mika-agent/src/agent.rs` (test_format_callback_framing_truncation_utf8_safe)

## Acceptance Criteria

- [ ] UTF-8 test exercises the `while !is_char_boundary` walk-back loop
- [ ] Test still passes (no panic, truncation occurs)
