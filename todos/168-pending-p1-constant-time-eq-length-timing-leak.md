---
status: pending
priority: p1
issue_id: "168"
tags: [code-review, security]
---

# Fix constant_time_eq Length Timing Leak

## Problem Statement
The simplified `constant_time_eq` function uses `subtle::ct_eq` directly on byte slices. The `subtle` crate's `ConstantTimeEq` for `[T]` explicitly short-circuits on length — returning `Choice::from(0)` immediately when slices differ in length. An attacker can determine the exact length of the `webhook_secret` by measuring response time differences when submitting tokens of varying lengths, reducing brute-force search space.

## Findings
- **Security sentinel**: HIGH severity — `webhook_secret` is exposed to the internet via Telegram webhook endpoint. Remote attacker can probe length.
- **Performance oracle**: Acceptable for fixed-length high-entropy tokens, but leaks length for variable-length inputs.
- **Architecture strategist**: The simplification is honest about what `subtle` provides. The old code was misleading.
- **Agent auth has same issue**: `crates/mika-agent/src/server/auth.rs:20` uses identical pattern.

## Proposed Solutions

### Option A: Enforce fixed-length tokens at startup (Recommended)
Validate in `GatewaySettings::load()` and `Settings::load()` that `webhook_secret` and `internal_token` are always a known fixed length (e.g., 64 hex chars). Length leak becomes moot since length is public.
- **Effort**: Small (15 min)
- **Risk**: None — tokens that don't match format are rejected at startup

### Option B: HMAC-based comparison
Hash both tokens with SHA-256 before comparing, producing fixed-length digests. Eliminates length leakage entirely.
- **Effort**: Small (20 min)
- **Risk**: Low — adds a dependency on a hash crate

### Option C: Restore length check before ct_eq
```rust
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    bool::from(a.ct_eq(b))
}
```
This is functionally identical to what `subtle` does internally, so it doesn't improve timing. Only useful if combined with Option A.
- **Effort**: Trivial (5 min)
- **Risk**: None, but doesn't fully solve the problem alone

## Technical Details
- **Affected files**: `crates/mika-gateway/src/routes.rs:427-429`, `crates/mika-agent/src/server/auth.rs:20`

## Acceptance Criteria
- [ ] Tokens validated for expected length at startup (or HMAC comparison used)
- [ ] Same fix applied to both gateway and agent auth paths
- [ ] No timing side-channel on token length

## Work Log
- 2026-02-24: Created from code review of commit 9de9ba6

## Resources
- Commit: 9de9ba6
- subtle crate source: `ConstantTimeEq for [T]` short-circuits on length
