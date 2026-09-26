---
status: complete
priority: p2
issue_id: "720"
tags: [code-review, security]
---

# Add range validation for `expires_in` from token endpoint

## Problem Statement

The `expires_in` field from Anthropic's token endpoint response is used directly without validation. A malicious/buggy server returning a negative or extremely large value would cause tokens to appear permanently expired or permanently valid.

## Findings

- **Source**: security-sentinel (F12, Low)
- **Location**: `crates/mika-common/src/oauth.rs` line 210, 456
- **Evidence**: `chrono::Duration::seconds(token_response.expires_in)` — no bounds check

## Proposed Solutions

### Option A: Add sanity range check (Recommended)
- Validate `expires_in > 0 && expires_in <= 86400 * 30` (max 30 days)
- Bail with clear error if out of range
- Apply to both `exchange_code()` and `refresh_tokens()`
- **Effort**: Small (4 lines)
- **Risk**: None

## Acceptance Criteria

- [ ] `expires_in <= 0` produces a clear error
- [ ] `expires_in > 30 days` produces a clear error
- [ ] Range check applied to both exchange and refresh paths
- [ ] Test added for invalid expires_in
